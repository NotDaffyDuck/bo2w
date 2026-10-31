"""Writes third_party/wgpu-hal: the crates.io release of wgpu-hal 29.0.4 with
acquire-wait.diff (next to this script) applied on top.

Cargo.toml points wgpu-hal at that folder through [patch.crates-io]. The folder
is committed, so a fresh clone builds as it is; run this again whenever
acquire-wait.diff changes, and commit the result:

    python third_party/wgpu-hal-patch/prepare.py

The .crate comes from cargo's download cache when it is there, otherwise from
static.crates.io; either way its sha256 must equal CRATE_SHA256 below.
"""
import hashlib, io, os, pathlib, shutil, subprocess, sys, tarfile, tempfile, urllib.request

NAME, VERSION = "wgpu-hal", "29.0.4"
CRATE_SHA256 = "97ace1c17727311c22a46e4e3faf56ea6de81af99dcc839bdfb54857b94d448d"
URL = f"https://static.crates.io/crates/{NAME}/{NAME}-{VERSION}.crate"

here = pathlib.Path(__file__).resolve().parent
root = here.parent.parent
diff = here / "acquire-wait.diff"
out = root / "third_party" / NAME
stamp_name = ".bo2mp-prepared"


def crate_bytes():
    cargo_home = pathlib.Path(os.environ.get("CARGO_HOME") or pathlib.Path.home() / ".cargo")
    for cached in sorted((cargo_home / "registry" / "cache").glob(f"*/{NAME}-{VERSION}.crate")):
        data = cached.read_bytes()
        if hashlib.sha256(data).hexdigest() == CRATE_SHA256:
            return data
    print(f"downloading {URL}")
    with urllib.request.urlopen(URL) as reply:
        data = reply.read()
    got = hashlib.sha256(data).hexdigest()
    if got != CRATE_SHA256:
        sys.exit(f"{URL}: sha256 {got}, expected {CRATE_SHA256}")
    return data


def main():
    want = CRATE_SHA256 + " " + hashlib.sha256(diff.read_bytes()).hexdigest()
    stamp = out / stamp_name
    if out.exists():
        if not stamp.is_file():
            sys.exit(f"{out} exists but was not written by this script; move it away and run again")
        if stamp.read_text().strip() == want:
            print(f"{out} is up to date")
            return
    work = pathlib.Path(tempfile.mkdtemp(prefix=".wgpu-hal-", dir=out.parent))
    try:
        with tarfile.open(fileobj=io.BytesIO(crate_bytes()), mode="r:gz") as tar:
            tar.extractall(work, filter="data")
        unpacked = work / f"{NAME}-{VERSION}"
        subprocess.run(["git", "apply", "--unsafe-paths", f"--directory={unpacked.as_posix()}", diff.as_posix()],
                       cwd=root, check=True)
        (unpacked / stamp_name).write_text(want + "\n")
        if out.exists():
            shutil.rmtree(out)
        unpacked.rename(out)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    print(f"wrote {out} ({NAME} {VERSION} + {diff.name})")


main()
