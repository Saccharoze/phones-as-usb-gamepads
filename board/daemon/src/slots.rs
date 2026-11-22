//! Who owns which pad. Pure logic: time comes in as an argument so the
//! rules are testable without sleeping.

use std::net::SocketAddr;
use std::time::Duration;

use proto::ErrorCode;
use tokio::time::Instant;

#[derive(Debug, Clone)]
pub struct Session {
    pub token: u64,
    pub addr: SocketAddr,
    pub last_seen: Instant,
    /// None right after HELLO, so a resumed phone may restart its counter.
    pub last_seq: Option<u16>,
    pub rtt_p50_ms: Option<u16>,
    pub rtt_p95_ms: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    New,
    Resumed,
    /// The same phone sent HELLO again before our WELCOME reached it.
    Repeated,
    TakenOver,
}

pub struct Slots {
    sessions: Vec<Option<Session>>,
    pin: Option<u16>,
    reservation: Duration,
}

impl Slots {
    pub fn new(count: usize, pin: Option<u16>, reservation: Duration) -> Slots {
        Slots { sessions: vec![None; count], pin, reservation }
    }

    pub fn count(&self) -> usize {
        self.sessions.len()
    }

    pub fn pin_required(&self) -> bool {
        self.pin.is_some()
    }

    /// `slot` is 1-based as on the wire. `new_token` is only used when a new
    /// session starts; passing it in keeps randomness out of this module.
    pub fn claim(
        &mut self,
        slot: u8,
        token: u64,
        pin: Option<u16>,
        addr: SocketAddr,
        now: Instant,
        new_token: u64,
    ) -> Result<(u64, Claim), ErrorCode> {
        let index = self.index(slot).ok_or(ErrorCode::NoSuchSlot)?;
        self.expire(now);
        let current = self.sessions[index].as_mut();
        if let Some(session) = current {
            if token != 0 && session.token == token {
                session.addr = addr;
                session.last_seen = now;
                session.last_seq = None;
                return Ok((token, Claim::Resumed));
            }
        }
        self.check_pin(pin)?;
        let kind = match self.sessions[index].as_mut() {
            None => Claim::New,
            // A HELLO retried while our WELCOME was in flight comes from the
            // same address. A new token would refuse that phone its own slot
            // (no PIN) or take the slot over from itself (PIN).
            Some(session) if session.addr == addr => {
                session.last_seen = now;
                return Ok((session.token, Claim::Repeated));
            }
            Some(_) if self.pin.is_some() => Claim::TakenOver,
            Some(_) => return Err(ErrorCode::SlotTaken),
        };
        self.sessions[index] = Some(Session {
            token: new_token,
            addr,
            last_seen: now,
            last_seq: None,
            rtt_p50_ms: None,
            rtt_p95_ms: None,
        });
        Ok((new_token, kind))
    }

    /// Finds the session for `token`, marks it seen, and follows a new address.
    pub fn touch(&mut self, token: u64, addr: SocketAddr, now: Instant) -> Option<(usize, &mut Session)> {
        let (index, session) = self.find_mut(token)?;
        session.last_seen = now;
        session.addr = addr;
        Some((index, session))
    }

    pub fn release(&mut self, token: u64) -> Option<usize> {
        let (index, _) = self.find_mut(token)?;
        self.sessions[index] = None;
        Some(index)
    }

    /// Frees slots whose phone has been silent longer than the reservation.
    pub fn expire(&mut self, now: Instant) -> Vec<usize> {
        let mut freed = Vec::new();
        for (index, slot) in self.sessions.iter_mut().enumerate() {
            if slot.as_ref().is_some_and(|s| now.duration_since(s.last_seen) >= self.reservation) {
                *slot = None;
                freed.push(index);
            }
        }
        freed
    }

    pub fn get(&self, index: usize) -> Option<&Session> {
        self.sessions.get(index).and_then(Option::as_ref)
    }

    fn find_mut(&mut self, token: u64) -> Option<(usize, &mut Session)> {
        if token == 0 {
            return None;
        }
        self.sessions
            .iter_mut()
            .enumerate()
            .find_map(|(i, s)| s.as_mut().filter(|s| s.token == token).map(|s| (i, s)))
    }

    fn index(&self, slot: u8) -> Option<usize> {
        let index = (slot as usize).checked_sub(1)?;
        (index < self.sessions.len()).then_some(index)
    }

    fn check_pin(&self, given: Option<u16>) -> Result<(), ErrorCode> {
        match self.pin {
            Some(pin) if given != Some(pin) => Err(ErrorCode::BadPin),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESERVE: Duration = Duration::from_secs(120);

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([192, 168, 1, 10], port))
    }

    #[test]
    fn new_claim_then_resume_keeps_the_token() {
        let now = Instant::now();
        let mut slots = Slots::new(2, None, RESERVE);
        assert_eq!(slots.claim(1, 0, None, addr(1), now, 77), Ok((77, Claim::New)));
        assert_eq!(slots.claim(1, 77, None, addr(2), now, 99), Ok((77, Claim::Resumed)));
        assert_eq!(slots.get(0).unwrap().addr, addr(2), "replies follow the new address");
    }

    #[test]
    fn rejects_missing_slots() {
        let mut slots = Slots::new(2, None, RESERVE);
        let now = Instant::now();
        assert_eq!(slots.claim(0, 0, None, addr(1), now, 1), Err(ErrorCode::NoSuchSlot));
        assert_eq!(slots.claim(3, 0, None, addr(1), now, 1), Err(ErrorCode::NoSuchSlot));
    }

    #[test]
    fn held_slot_is_locked_without_pin() {
        let now = Instant::now();
        let mut slots = Slots::new(2, None, RESERVE);
        slots.claim(1, 0, None, addr(1), now, 5).unwrap();
        assert_eq!(slots.claim(1, 0, None, addr(2), now, 6), Err(ErrorCode::SlotTaken));
        assert_eq!(slots.claim(1, 1234, None, addr(2), now, 6), Err(ErrorCode::SlotTaken));
    }

    #[test]
    fn reservation_lasts_120_s_after_silence() {
        let start = Instant::now();
        let mut slots = Slots::new(1, None, RESERVE);
        slots.claim(1, 0, None, addr(1), start, 5).unwrap();
        let almost = start + RESERVE - Duration::from_millis(1);
        assert_eq!(slots.claim(1, 0, None, addr(2), almost, 6), Err(ErrorCode::SlotTaken));
        assert_eq!(slots.claim(1, 0, None, addr(2), start + RESERVE, 6), Ok((6, Claim::New)));
    }

    #[test]
    fn traffic_extends_the_reservation() {
        let start = Instant::now();
        let mut slots = Slots::new(1, None, RESERVE);
        slots.claim(1, 0, None, addr(1), start, 5).unwrap();
        slots.touch(5, addr(1), start + Duration::from_secs(100)).unwrap();
        assert!(slots.expire(start + Duration::from_secs(150)).is_empty());
        assert_eq!(slots.expire(start + Duration::from_secs(220)), vec![0]);
    }

    #[test]
    fn pin_guards_new_claims_and_allows_takeover() {
        let now = Instant::now();
        let mut slots = Slots::new(2, Some(4321), RESERVE);
        assert_eq!(slots.claim(1, 0, None, addr(1), now, 5), Err(ErrorCode::BadPin));
        assert_eq!(slots.claim(1, 0, Some(1), addr(1), now, 5), Err(ErrorCode::BadPin));
        assert_eq!(slots.claim(1, 0, Some(4321), addr(1), now, 5), Ok((5, Claim::New)));
        assert_eq!(slots.claim(1, 0, Some(4321), addr(2), now, 6), Ok((6, Claim::TakenOver)));
        assert!(slots.touch(5, addr(1), now).is_none(), "old token is dead after takeover");
        assert_eq!(slots.claim(1, 6, None, addr(2), now, 7), Ok((6, Claim::Resumed)), "resume needs no PIN");
    }

    #[test]
    fn a_retried_hello_gets_the_same_token() {
        let now = Instant::now();
        let mut slots = Slots::new(1, None, RESERVE);
        assert_eq!(slots.claim(1, 0, None, addr(1), now, 5), Ok((5, Claim::New)));
        assert_eq!(slots.claim(1, 0, None, addr(1), now, 6), Ok((5, Claim::Repeated)));
        let mut pinned = Slots::new(1, Some(4321), RESERVE);
        pinned.claim(1, 0, Some(4321), addr(1), now, 5).unwrap();
        assert_eq!(pinned.claim(1, 0, Some(4321), addr(1), now, 6), Ok((5, Claim::Repeated)), "no takeover of itself");
        assert_eq!(pinned.claim(1, 0, None, addr(1), now, 6), Err(ErrorCode::BadPin), "still a new claim: PIN needed");
    }

    #[test]
    fn release_frees_the_slot() {
        let now = Instant::now();
        let mut slots = Slots::new(1, None, RESERVE);
        slots.claim(1, 0, None, addr(1), now, 5).unwrap();
        assert_eq!(slots.release(5), Some(0));
        assert!(slots.get(0).is_none());
        assert_eq!(slots.release(5), None);
    }

    #[test]
    fn token_zero_never_matches() {
        let now = Instant::now();
        let mut slots = Slots::new(1, None, RESERVE);
        slots.claim(1, 0, None, addr(1), now, 5).unwrap();
        assert!(slots.touch(0, addr(1), now).is_none());
    }
}
