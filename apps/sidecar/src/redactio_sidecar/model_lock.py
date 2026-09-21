"""Native receipt locks shared by management and inference workers."""

from __future__ import annotations

import os
import stat
import sys
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path
from typing import Any, cast

from .ipc import EngineError
from .model_store import _safe_path


def _checked_path(root: Path, relative: str) -> Path:
    try:
        return _safe_path(root.absolute(), relative)
    except (OSError, ValueError) as error:
        raise EngineError("model_path_unsafe") from error


def _regular(path: Path) -> None:
    status = path.lstat()
    if (
        not stat.S_ISREG(status.st_mode)
        or status.st_nlink != 1
        or getattr(status, "st_file_attributes", 0) & 0x400
    ):
        raise EngineError("model_path_unsafe")


@contextmanager
def _native_lock(path: Path, *, create: bool, code: str, shared: bool = False) -> Iterator[Any]:
    _checked_path(path.parent, path.name)
    if path.exists():
        _regular(path)
    if sys.platform == "win32":
        import ctypes
        import msvcrt
        from ctypes import wintypes

        class Overlapped(ctypes.Structure):
            _fields_ = [
                ("Internal", ctypes.c_size_t),
                ("InternalHigh", ctypes.c_size_t),
                ("Offset", wintypes.DWORD),
                ("OffsetHigh", wintypes.DWORD),
                ("hEvent", wintypes.HANDLE),
            ]

        kernel = cast(Any, ctypes).WinDLL("kernel32", use_last_error=True)
        kernel.CreateFileW.argtypes = [
            wintypes.LPCWSTR,
            wintypes.DWORD,
            wintypes.DWORD,
            ctypes.c_void_p,
            wintypes.DWORD,
            wintypes.DWORD,
            wintypes.HANDLE,
        ]
        kernel.CreateFileW.restype = wintypes.HANDLE
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        kernel.LockFileEx.argtypes = [
            wintypes.HANDLE,
            wintypes.DWORD,
            wintypes.DWORD,
            wintypes.DWORD,
            wintypes.DWORD,
            ctypes.POINTER(Overlapped),
        ]
        kernel.UnlockFileEx.argtypes = [
            wintypes.HANDLE,
            wintypes.DWORD,
            wintypes.DWORD,
            wintypes.DWORD,
            ctypes.POINTER(Overlapped),
        ]
        # Share delete permits removal while this exclusive receipt lease remains held.
        handle = kernel.CreateFileW(
            str(path),
            0xC0000000 if create else 0x80000000,
            7,
            None,
            4 if create else 3,
            0x200080,
            None,
        )
        if handle == ctypes.c_void_p(-1).value:
            raise cast(Any, ctypes).WinError(cast(Any, ctypes).get_last_error())
        try:
            fd = cast(Any, msvcrt).open_osfhandle(handle, os.O_RDWR if create else os.O_RDONLY)
        except BaseException:
            kernel.CloseHandle(handle)
            raise
        with os.fdopen(fd, "r+b" if create else "rb") as stream:
            _regular(path)
            overlapped = Overlapped()
            if not kernel.LockFileEx(handle, 1 if shared else 3, 0, 1, 0, ctypes.byref(overlapped)):
                raise EngineError(code, retryable=True)
            try:
                _locked_path(stream, path)
                yield stream
            finally:
                kernel.UnlockFileEx(handle, 0, 1, 0, ctypes.byref(overlapped))
    else:
        import fcntl

        fd = os.open(
            path, os.O_NOFOLLOW | (os.O_RDWR | os.O_CREAT if create else os.O_RDONLY), 0o600
        )
        with os.fdopen(fd, "r+b" if create else "rb") as stream:
            _regular(path)
            if not os.path.samestat(os.fstat(fd), path.stat()):
                raise EngineError("model_path_unsafe")
            try:
                fcntl.flock(fd, (fcntl.LOCK_SH if shared else fcntl.LOCK_EX) | fcntl.LOCK_NB)
            except BlockingIOError as error:
                raise EngineError(code, retryable=True) from error
            try:
                _locked_path(stream, path)
                yield stream
            finally:
                fcntl.flock(fd, fcntl.LOCK_UN)


def _locked_path(stream: Any, path: Path) -> None:
    _checked_path(path.parent, path.name)
    _regular(path)
    if not os.path.samestat(os.fstat(stream.fileno()), path.stat()):
        raise EngineError("model_path_unsafe")
