"""macOS arm64 support, checkable on any OS: the committed binary's structure, platform detection, lookup, diagnostics."""
import hashlib
import os
import shutil
import subprocess
import sys

import pytest

from hillview import native

ROOT = os.path.join(os.path.dirname(__file__), "..")
DYLIB = os.path.join(ROOT, "prebuilt", "macos-arm64", "libhillview.dylib")
sys.path.insert(0, os.path.join(ROOT, "core", "tools"))
import macho_info  # noqa: E402


def test_prebuilt_dylib_is_a_loadable_looking_arm64_macos_library():
    i = macho_info.info(DYLIB)
    assert i["arm64"] and i["dylib"], "not an arm64 dylib"
    assert i["platform"] == 1 and i["minos"] == "11.0.0", (i["platform"], i["minos"])      # macOS, first arm64 release
    assert i["deps"] == ["/usr/lib/libSystem.B.dylib"], i["deps"]                          # self-contained (libc++ is static)
    assert i["signed"] and i["adhoc"] and i["signature_ends_at_eof"], "arm64 macOS refuses unsigned libraries"
    assert i["install_name"] == "@rpath/libhillview.dylib"


def _nm(*flags):
    out = subprocess.run(["llvm-nm", *flags, DYLIB], capture_output=True, text=True, check=True).stdout
    return [l.split()[-1] for l in out.splitlines() if l.strip()]


@pytest.mark.skipif(not shutil.which("llvm-nm"), reason="llvm-nm not installed")
def test_prebuilt_dylib_exports_only_the_public_api():
    exported = sorted(l.split()[-1] for l in subprocess.run(["llvm-nm", "-gU", DYLIB], capture_output=True, text=True,
                                                            check=True).stdout.splitlines() if " T " in l or " S " in l or " A " in l or " D " in l or " B " in l)
    api = ["_hv_version", "_hv_process_f32", "_hv_process_u8", "_hv_blend_u8", "_hv_hash01", "_hv_mod_source"]
    linker_synthesized = ["___dso_handle", "__mh_dylib_header"]          # added by every Mach-O dylib link
    assert exported == sorted(api + linker_synthesized), exported


@pytest.mark.skipif(not shutil.which("llvm-nm"), reason="llvm-nm not installed")
def test_prebuilt_dylib_uses_no_cpp_runtime():
    """A static libc++ would export operator new/delete from the dylib, where dyld can coalesce them with the host's own."""
    undefined = _nm("-u")
    bad = [u for u in undefined if u.startswith(("__Z", "___cxa", "__Unwind", "___gxx", "__ZNSt"))]
    assert not bad, bad
    assert not [e for e in _nm("-gU") if e.startswith("__Z")], "C++ runtime symbols are exported"


def test_prebuilt_checksum_matches_file():
    sums = open(os.path.join(os.path.dirname(DYLIB), "SHA256SUMS")).read().split()
    assert sums[1] == "libhillview.dylib"
    assert sums[0] == hashlib.sha256(open(DYLIB, "rb").read()).hexdigest()


@pytest.mark.parametrize("plat,machine,want", [
    ("darwin", "arm64", "macos-arm64"), ("darwin", "x86_64", "macos-x86_64"),
    ("linux", "x86_64", "linux-x86_64"), ("linux", "aarch64", "linux-arm64"), ("win32", "AMD64", "windows-x86_64"),
])
def test_platform_tag(monkeypatch, plat, machine, want):
    monkeypatch.setattr(sys, "platform", plat)
    monkeypatch.setattr(native.platform, "machine", lambda: machine)
    assert native.platform_tag() == want


def test_mac_arm64_lookup_finds_prebuilt_dylib(monkeypatch):
    monkeypatch.setattr(sys, "platform", "darwin")
    monkeypatch.setattr(native.platform, "machine", lambda: "arm64")
    monkeypatch.delenv("HILLVIEW_LIB", raising=False)
    cands = [os.path.normpath(c) for c in native._candidates()]
    assert os.path.normpath(DYLIB) in cands
    assert all(os.path.basename(c).startswith("libhillview") and ".so" not in c for c in cands)


def test_load_failure_is_reported_not_silent(monkeypatch):
    """Simulates dlopen refusing a library (wrong architecture here; on a Mac it would be Gatekeeper/signature)."""
    monkeypatch.setattr(native, "_candidates", lambda: iter([DYLIB]))
    try:
        if sys.platform == "darwin" and native.platform_tag() == "macos-arm64":
            pytest.skip("on a real Apple Silicon Mac this library is supposed to load")
        assert native.load(force=True) is None
        msg = native.describe()
        assert "unavailable" in msg and "libhillview.dylib" in msg and "numpy" in msg, msg
    finally:
        monkeypatch.undo()
        native.load(force=True)   # restore the real state for other tests
