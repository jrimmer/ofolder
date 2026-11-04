"""Gives the nested compositor a pointer, and holds it until killed.

A nested compositor offers its clients a pointer only when the real seat it sits on has one. A
desktop with no mouse attached (a laptop on a dock, a remote session) has none, and then no click
reaches a dialog. This creates a virtual pointer inside the nested compositor with the wlroots
`zwlr_virtual_pointer_manager_v1` protocol, speaking the Wayland wire format directly so nothing
needs installing. The device exists for as long as this process holds the connection.
"""
import os
import socket
import struct
import sys
import time


def pad(n):
    return (n + 3) & ~3


def message(obj, opcode, payload=b""):
    return struct.pack("<II", obj, ((8 + len(payload)) << 16) | opcode) + payload


def string(s):
    b = s.encode() + b"\0"
    return struct.pack("<I", len(b)) + b + b"\0" * (pad(len(b)) - len(b))


def events(sock, timeout=5):
    """Yields (object, opcode, payload) until the socket has nothing more for a moment."""
    sock.settimeout(timeout)
    buf = b""
    while True:
        try:
            data = sock.recv(65536)
        except socket.timeout:
            return
        if not data:
            return
        buf += data
        while len(buf) >= 8:
            obj, word = struct.unpack("<II", buf[:8])
            size, opcode = word >> 16, word & 0xFFFF
            if len(buf) < size:
                break
            yield obj, opcode, buf[8:size]
            buf = buf[size:]


def main():
    path = os.path.join(os.environ["XDG_RUNTIME_DIR"], os.environ.get("WAYLAND_DISPLAY", "wayland-1"))
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.connect(path)
    sock.sendall(message(1, 1, struct.pack("<I", 2)))  # wl_display.get_registry -> object 2
    sock.sendall(message(1, 0, struct.pack("<I", 3)))  # wl_display.sync -> callback 3
    globals_ = {}
    for obj, opcode, payload in events(sock):
        if obj == 2 and opcode == 0:  # wl_registry.global
            name, length = struct.unpack("<II", payload[:8])
            interface = payload[8 : 8 + length - 1].decode()
            version = struct.unpack("<I", payload[8 + pad(length) : 12 + pad(length)])[0]
            globals_[interface] = (name, version)
        if obj == 3:  # the sync is done: every global has been announced
            break
    seat, manager = globals_.get("wl_seat"), globals_.get("zwlr_virtual_pointer_manager_v1")
    if not seat or not manager:
        print("no virtual pointer support in the compositor", file=sys.stderr)
        sys.exit(2)
    # wl_registry.bind(name, interface, version, new_id): seat -> 4, manager -> 5
    for obj_id, (interface, (name, version)) in ((4, ("wl_seat", seat)), (5, ("zwlr_virtual_pointer_manager_v1", manager))):
        sock.sendall(message(2, 0, struct.pack("<I", name) + string(interface) + struct.pack("<II", min(version, 7 if obj_id == 4 else 2), obj_id)))
    # zwlr_virtual_pointer_manager_v1.create_virtual_pointer(seat, id) -> pointer 6
    sock.sendall(message(5, 0, struct.pack("<II", 4, 6)))
    sock.sendall(message(1, 0, struct.pack("<I", 7)))  # sync again, to know it was accepted
    for obj, _, _ in events(sock):
        if obj == 7:
            break
    print("ready", flush=True)
    if len(sys.argv) < 2:
        while True:
            time.sleep(3600)
    # Commands arrive as lines in a named pipe, opened for reading and writing so that it never
    # reports the end of the file when a writer goes away: "scroll N" turns the wheel N notches
    # (negative is up). The pointer must already be over the window, which the caller arranges.
    fd = os.open(sys.argv[1], os.O_RDWR)
    pending = b""
    while True:
        pending += os.read(fd, 4096)
        while b"\n" in pending:
            line, pending = pending.split(b"\n", 1)
            words = line.split()
            if len(words) == 2 and words[0] == b"scroll":
                notches = int(words[1])
                for _ in range(abs(notches)):
                    now = int(time.time() * 1000) & 0xFFFFFFFF
                    sock.sendall(message(6, 5, struct.pack("<I", 0)))  # axis_source: wheel
                    step = 15 * 256 * (1 if notches > 0 else -1)
                    sock.sendall(message(6, 7, struct.pack("<IIii", now, 0, step, 1 if notches > 0 else -1)))
                    sock.sendall(message(6, 4))  # frame
                    time.sleep(0.05)


main()
