"""Check the real Windows GUI launcher with private consoles and explicit handle inheritance."""
from __future__ import annotations

import argparse
import ctypes
from ctypes import wintypes as w
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import time


class Coord(ctypes.Structure):
    """Win32 console coordinates are signed 16-bit values, passed by value."""
    _fields_ = [("x", w.SHORT), ("y", w.SHORT)]


class Rect(ctypes.Structure):
    """Console window extent in character cells."""
    _fields_ = [(name, w.SHORT) for name in ("left", "top", "right", "bottom")]


class BufferInfo(ctypes.Structure):
    """Read enough screen-buffer metadata to capture its complete text."""
    _fields_ = [("size", Coord), ("cursor", Coord), ("attributes", w.WORD),
                ("window", Rect), ("maximum", Coord)]


class StartupInfo(ctypes.Structure):
    """CreateProcess startup data, including deliberately missing standard handles."""
    _fields_ = [("cb", w.DWORD), ("reserved", w.LPWSTR), ("desktop", w.LPWSTR),
                ("title", w.LPWSTR), *[(name, w.DWORD) for name in
                ("x", "y", "x_size", "y_size", "x_chars", "y_chars", "fill", "flags")],
                ("show", w.WORD), ("reserved_size", w.WORD), ("reserved_bytes", w.LPBYTE),
                ("stdin", w.HANDLE), ("stdout", w.HANDLE), ("stderr", w.HANDLE)]


class ProcessInfo(ctypes.Structure):
    """Only these task-owned process handles may be terminated or closed."""
    _fields_ = [("process", w.HANDLE), ("thread", w.HANDLE), ("pid", w.DWORD), ("tid", w.DWORD)]


def kernel():
    """Declare pointer-sized signatures; ctypes' default integer return truncates handles."""
    k = ctypes.WinDLL("kernel32", use_last_error=True)
    declarations = {
        "FreeConsole": ([], w.BOOL), "AllocConsole": ([], w.BOOL),
        "AttachConsole": ([w.DWORD], w.BOOL),
        "CreateFileW": ([w.LPCWSTR, w.DWORD, w.DWORD, w.LPVOID, w.DWORD, w.DWORD, w.HANDLE], w.HANDLE),
        "GetConsoleScreenBufferInfo": ([w.HANDLE, ctypes.POINTER(BufferInfo)], w.BOOL),
        "ReadConsoleOutputCharacterW": ([w.HANDLE, w.LPWSTR, w.DWORD, Coord, ctypes.POINTER(w.DWORD)], w.BOOL),
        "CreateProcessW": ([w.LPCWSTR, w.LPWSTR, w.LPVOID, w.LPVOID, w.BOOL, w.DWORD,
                            w.LPVOID, w.LPCWSTR, ctypes.POINTER(StartupInfo), ctypes.POINTER(ProcessInfo)], w.BOOL),
        "GetExitCodeProcess": ([w.HANDLE, ctypes.POINTER(w.DWORD)], w.BOOL),
        "WaitForSingleObject": ([w.HANDLE, w.DWORD], w.DWORD),
        "TerminateProcess": ([w.HANDLE, w.UINT], w.BOOL),
        "CloseHandle": ([w.HANDLE], w.BOOL),
    }
    for name, (args, result) in declarations.items():
        function = getattr(k, name)
        function.argtypes, function.restype = args, result
    return k


def checked(ok):
    """Report a native setup failure rather than passing an unexercised scenario."""
    if not ok:
        raise ctypes.WinError(ctypes.get_last_error())


def console_text(k, handle):
    """Read the private console without relying on PowerShell transcript capture."""
    info = BufferInfo()
    checked(k.GetConsoleScreenBufferInfo(handle, ctypes.byref(info)))
    count = min(info.size.x * info.size.y, 32768)
    output = ctypes.create_unicode_buffer(count + 1)
    read = w.DWORD()
    checked(k.ReadConsoleOutputCharacterW(handle, output, count, Coord(0, 0), ctypes.byref(read)))
    return output[:read.value]


def check_case(executable: Path, mode: str, kind: str):
    """Use one helper process per case so global console changes never affect the runner."""
    import msvcrt

    k = kernel()
    k.FreeConsole()
    console = None
    if mode != "no-console":
        checked(k.AllocConsole())
        console = k.CreateFileW("CONOUT$", 0xC0000000, 3, None, 3, 0, None)
        checked(console not in (None, ctypes.c_void_p(-1).value))
    try:
        with tempfile.TemporaryDirectory(prefix="pcbcraft-startup-console-") as directory:
            root = Path(directory)
            # Isolate preferences, GPU cache and recents from the runner's profile.
            for name in ("APPDATA", "LOCALAPPDATA", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"):
                os.environ[name] = str(root)
            expected = "PCBCraft has no options · usage: pcbcraft [PROJECT]" if kind == "usage" else "Could not open"
            target = root / "missing-電路.pc"
            arguments = [str(executable), "--help" if kind == "usage" else str(target)]
            startup, process = StartupInfo(), ProcessInfo()
            startup.cb = ctypes.sizeof(startup)
            capture = root / "redirected.txt"
            with open(os.devnull if mode == "null" else capture, "wb") as redirected:
                inherited = mode in {"redirected", "stdout-only", "null"}
                if inherited:
                    startup.flags = 0x100  # STARTF_USESTDHANDLES
                    handle = msvcrt.get_osfhandle(redirected.fileno())
                    os.set_handle_inheritable(handle, True)
                    if mode == "stdout-only":
                        startup.stdout = handle
                    else:
                        startup.stderr = handle
                # No inheritance means the GUI executable has no stderr; stdout-only
                # specifically exercises an empty stderr retained by STARTF_USESTDHANDLES.
                command = ctypes.create_unicode_buffer(subprocess.list2cmdline(arguments))
                checked(k.CreateProcessW(str(executable), command, None, None, inherited, 0,
                                        None, str(root), ctypes.byref(startup), ctypes.byref(process)))
                try:
                    deadline = time.monotonic() + 30
                    text = ""
                    while time.monotonic() < deadline:
                        text = capture.read_bytes().decode("utf-8") if mode == "redirected" else (
                            console_text(k, console) if console else "")
                        if (expected in text and (kind == "usage" or target.name in text)) or k.WaitForSingleObject(process.process, 0) == 0:
                            break
                        if mode in {"no-console", "null"} and time.monotonic() > deadline - 25:
                            break
                        time.sleep(0.05)
                    if kind == "usage":
                        assert k.WaitForSingleObject(process.process, 5000) == 0, "usage did not exit"
                        code = w.DWORD()
                        checked(k.GetExitCodeProcess(process.process, ctypes.byref(code)))
                        assert code.value == 2, code.value
                        text = capture.read_bytes().decode("utf-8") if mode == "redirected" else (
                            console_text(k, console) if console else "")
                    if mode == "no-console":
                        if kind == "open":
                            assert k.WaitForSingleObject(process.process, 0) == 258, "failed open did not keep Home alive"
                            assert not k.AttachConsole(process.pid), "GUI launch allocated a console"
                        assert not capture.read_bytes()
                    elif mode == "null":
                        assert expected not in console_text(k, console), "explicit NUL stderr was overridden"
                        if kind == "open":
                            assert k.WaitForSingleObject(process.process, 0) == 258, "failed open did not keep Home alive"
                    else:
                        assert text.count(expected) == 1, repr(text)
                        if kind == "open":
                            assert "missing-電路.pc" in text, repr(text)
                        if mode == "stdout-only":
                            assert not capture.read_bytes(), "stderr was redirected to stdout"
                        if mode == "redirected":
                            assert expected not in console_text(k, console), "redirected stderr also leaked to console"
                finally:
                    if k.WaitForSingleObject(process.process, 0) == 258:
                        checked(k.TerminateProcess(process.process, 0))
                        checked(k.WaitForSingleObject(process.process, 5000) == 0)
                    k.CloseHandle(process.thread)
                    k.CloseHandle(process.process)
            return {"mode": mode, "kind": kind, "passed": True}
    finally:
        if console:
            k.CloseHandle(console)
        k.FreeConsole()


def require_gui_subsystem(executable: Path):
    """A console-subsystem executable would hide the regression this test must catch."""
    data = executable.read_bytes()
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    assert data[pe:pe + 4] == b"PE\0\0"
    assert struct.unpack_from("<H", data, pe + 24 + 68)[0] == 2, "expected Windows GUI subsystem"


def main():
    """Fail on a stalled or missing scenario and retain compact per-case results."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--case", choices=("terminal", "redirected", "stdout-only", "null", "no-console"))
    parser.add_argument("--kind", choices=("usage", "open"))
    args = parser.parse_args()
    if args.case and not args.kind:
        parser.error("--case requires --kind")
    if sys.platform != "win32":
        raise SystemExit("This check requires Windows.")
    executable = args.executable.resolve(strict=True)
    require_gui_subsystem(executable)
    if args.case:
        print(json.dumps(check_case(executable, args.case, args.kind)))
    else:
        for mode in ("terminal", "redirected", "stdout-only", "null", "no-console"):
            for kind in ("usage", "open"):
                command = [sys.executable, "-B", __file__, "--executable", str(executable), "--case", mode, "--kind", kind]
                result = subprocess.run(command, check=True, text=True, capture_output=True,
                                        timeout=45, creationflags=subprocess.CREATE_NO_WINDOW)
                print(result.stdout.strip(), flush=True)


if __name__ == "__main__":
    main()
