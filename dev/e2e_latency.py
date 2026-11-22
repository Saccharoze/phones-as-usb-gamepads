"""End-to-end check from the dev PC: UDP to stonesd on the board, read back the pad's evdev events.
Both timestamps come from this PC's CLOCK_REALTIME (evdev default), so no clock sync is needed."""
import os, select, socket, statistics, struct, sys, time

BOARD = (sys.argv[1], 40400)
EVDEV = sys.argv[2]           # pad 1's /dev/input/eventN on this PC
SAMPLES = int(sys.argv[3])
EV = struct.Struct("llHHi")

def hdr(t): return b"SS\x01" + bytes([t])
def report(buttons): return struct.pack("<HBhhhhBB", buttons, 8, 0, 0, 0, 0, 0, 0)

sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); sock.settimeout(1)
sock.sendto(hdr(0x01) + struct.pack("<BQH", 1, 0, 0xFFFF), BOARD)
reply, _ = sock.recvfrom(64)
assert reply[3] == 0x02, f"no WELCOME: {reply.hex()}"
token = struct.unpack("<Q", reply[5:13])[0]
fd = os.open(EVDEV, os.O_RDONLY | os.O_NONBLOCK)
while select.select([fd], [], [], 0.05)[0]: os.read(fd, 4096)   # drain

def wait_key(value, timeout=1.0):
    end = time.time() + timeout
    while time.time() < end:
        if not select.select([fd], [], [], 0.05)[0]: continue
        data = os.read(fd, EV.size * 64)
        for i in range(0, len(data), EV.size):
            sec, usec, typ, code, val = EV.unpack(data[i:i + EV.size])
            if typ == 1 and code == 0x130 and val == value:
                return sec + usec / 1e6
    return None

seq, lat, lost = 0, [], 0
for n in range(SAMPLES):
    pressed = n % 2 == 0
    seq += 1
    t0 = time.time()
    sock.sendto(hdr(0x10) + struct.pack("<QH", token, seq) + report(1 if pressed else 0), BOARD)
    t1 = wait_key(1 if pressed else 0)
    if t1 is None: lost += 1
    else: lat.append((t1 - t0) * 1000)
    time.sleep(0.02)

q = statistics.quantiles(lat, n=100)
print(f"samples {len(lat)}/{SAMPLES} lost {lost}  e2e ms: min {min(lat):.2f} p50 {q[49]:.2f} p95 {q[94]:.2f} max {max(lat):.2f}")

# Fail-safe: hold A, go silent, time the release seen by the PC.
seq += 1
t0 = time.time()
sock.sendto(hdr(0x10) + struct.pack("<QH", token, seq) + report(1), BOARD)
t_press = wait_key(1); t_release = wait_key(0, timeout=2)
print(f"fail-safe: press seen after {(t_press - t0)*1000:.1f} ms, released {(t_release - t_press)*1000:.0f} ms after the press with no packets")
sock.sendto(hdr(0x30) + struct.pack("<Q", token), BOARD)
