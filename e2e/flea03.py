"""Flea 0.3.x for the 0.3 half of the suite: a pinned, hash-checked release unpacked into a cache.

Flea 0.3's picker is a Quickshell window, 0.4's is its own GPUI window. The suite runs the same
checks against both, so a Flea update that breaks either one is caught here. Nothing is installed:
the release is unpacked under ~/.cache and used only through FLEA_BIN and FLEA_UI in the nested
session.
"""
import hashlib
import os
import tarfile
import urllib.request

VERSION = "0.3.12"
NAME = "flea-v%s-linux-x86_64.tar.gz" % VERSION
URL = "https://github.com/thisisgm/flea/releases/download/v%s/%s" % (VERSION, NAME)
SHA256 = "c229951fdfb43661f3496d970ec7c335c78320dc0ea05f8d1448aedbb9f3b866"
CACHE = os.path.join(os.environ.get("XDG_CACHE_HOME") or os.path.expanduser("~/.cache"), "ofolder-e2e")
SHELL = "/usr/share/omarchy/shell"


def ensure():
    """The FLEA_BIN and FLEA_UI environment for 0.3, or None (and why) when it cannot be had."""
    root = os.path.join(CACHE, "flea-%s-linux-x86_64" % VERSION)
    tarball = os.path.join(CACHE, NAME)
    if not os.path.exists(os.path.join(root, "flea")):
        os.makedirs(CACHE, exist_ok=True)
        if not os.path.exists(tarball):
            urllib.request.urlretrieve(URL, tarball + ".part")
            os.replace(tarball + ".part", tarball)
        got = hashlib.sha256(open(tarball, "rb").read()).hexdigest()
        if got != SHA256:
            os.remove(tarball)
            return None, "the downloaded Flea %s does not match the pinned hash (%s)" % (VERSION, got)
        with tarfile.open(tarball) as t:
            t.extractall(CACHE, filter="data")
    boot = os.path.join(root, "ui", "boot")
    for name in ("Commons", "Ui"):
        link = os.path.join(boot, name)
        if not os.path.lexists(link):
            if not os.path.isdir(os.path.join(SHELL, name)):
                return None, "Omarchy's shell modules are missing (%s/%s)" % (SHELL, name)
            os.symlink(os.path.join(SHELL, name), link)
    return {"FLEA_BIN": os.path.join(root, "flea"), "FLEA_UI": os.path.join(root, "ui")}, ""
