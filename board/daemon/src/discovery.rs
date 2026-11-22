//! mDNS advertisement, so phones find the board without typing an address.

use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use proto::frame::SERVICE_TYPE;

/// Keeps advertising until dropped.
pub struct Advertisement {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Drop for Advertisement {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

pub fn advertise(port: u16, pad_count: u8, pin_required: bool) -> Result<Advertisement> {
    let daemon = ServiceDaemon::new().context("starting mDNS")?;
    let host = hostname();
    let instance = format!("Sending Stones on {host}");
    let pads = pad_count.to_string();
    let pin = if pin_required { "1" } else { "0" };
    let properties = [("v", "1"), ("pads", pads.as_str()), ("pin", pin)];
    let info = ServiceInfo::new(SERVICE_TYPE, &instance, &format!("{host}.local."), "", port, &properties[..])
        .context("building mDNS record")?
        .enable_addr_auto();
    let fullname = info.get_fullname().to_owned();
    daemon.register(info).context("registering mDNS service")?;
    tracing::info!(service = %fullname, port, "advertising over mDNS");
    Ok(Advertisement { daemon, fullname })
}

fn hostname() -> String {
    let from_os = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok());
    from_os
        .map(|h| h.trim().to_owned())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "sending-stones".to_owned())
}
