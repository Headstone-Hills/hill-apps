import os
import shutil
import subprocess
import pytest

CORE = os.path.join(os.path.dirname(__file__), "..", "core")


@pytest.fixture(scope="session")
def native_lib():
    """The native core if present, building it once if a compiler is available; else None (tests skip)."""
    from hillview import native
    if not native.available() and shutil.which("cmake") and (shutil.which("g++") or shutil.which("clang++")):
        subprocess.run([os.path.join(CORE, "build.sh")], check=False, capture_output=True)
        native.load(force=True)
    return native.load()
