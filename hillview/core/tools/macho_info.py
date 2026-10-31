"""Inspect a Mach-O dylib without Apple tools: arch, install name, min macOS, dependencies, code signature.

CLI:  python3 core/tools/macho_info.py libhillview.dylib
API:  info(path) -> dict   (used by tests to guard the committed prebuilt binary)
"""
import struct
import sys

LC_LOAD_DYLIB, LC_ID_DYLIB, LC_CODE_SIGNATURE, LC_BUILD_VERSION = 0x0C, 0x0D, 0x1D, 0x32
CPU_ARM64, MH_DYLIB = 0x0100000C, 6


def info(path):
    d = open(path, "rb").read()
    magic, cputype, _sub, ftype, ncmds, _size, _flags, _res = struct.unpack_from("<IiiIIIII", d, 0)
    r = {"magic": magic, "cputype": cputype, "filetype": ftype, "arm64": cputype == CPU_ARM64, "dylib": ftype == MH_DYLIB,
         "deps": [], "install_name": None, "minos": None, "platform": None, "signed": False, "adhoc": False,
         "signature_ends_at_eof": False}
    off = 32
    for _ in range(ncmds):
        cmd, size = struct.unpack_from("<II", d, off)
        if cmd in (LC_LOAD_DYLIB, LC_ID_DYLIB):
            nameoff = struct.unpack_from("<I", d, off + 8)[0]
            name = d[off + nameoff: off + size].split(b"\0")[0].decode()
            if cmd == LC_ID_DYLIB:
                r["install_name"] = name
            else:
                r["deps"].append(name)
        elif cmd == LC_BUILD_VERSION:
            plat, minos, _sdk, _nt = struct.unpack_from("<IIII", d, off + 8)
            r["platform"], r["minos"] = plat, f"{minos >> 16}.{(minos >> 8) & 255}.{minos & 255}"
        elif cmd == LC_CODE_SIGNATURE:
            dataoff, datasize = struct.unpack_from("<II", d, off + 8)
            r["signed"] = struct.unpack_from(">I", d, dataoff)[0] == 0xFADE0CC0       # embedded SuperBlob
            r["signature_ends_at_eof"] = dataoff + datasize == len(d)
            _m, _l, count = struct.unpack_from(">III", d, dataoff)
            for i in range(count):
                t, o = struct.unpack_from(">II", d, dataoff + 12 + 8 * i)
                if t == 0:                                                          # CodeDirectory
                    r["adhoc"] = bool(struct.unpack_from(">IIII", d, dataoff + o)[3] & 2)
        off += size
    return r


if __name__ == "__main__":
    i = info(sys.argv[1])
    print(f"arch: {'arm64' if i['arm64'] else hex(i['cputype'])}   type: {'dylib' if i['dylib'] else i['filetype']}   "
          f"platform: {'macOS' if i['platform'] == 1 else i['platform']}   min OS: {i['minos']}")
    print(f"install name: {i['install_name']}   depends on: {', '.join(i['deps']) or '(nothing)'}")
    print(f"code signature: {'ad-hoc' if i['adhoc'] else ('present' if i['signed'] else 'MISSING (arm64 macOS will not load it)')}"
          f"{'' if i['signature_ends_at_eof'] or not i['signed'] else '  (WARNING: signature does not end at EOF)'}")
