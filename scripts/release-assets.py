"""Prepare the versioned GitHub Releases feed from native CI builds."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[1]
VERSION = json.loads((ROOT / "tauri.conf.json").read_text())["version"]
NAMES = {"darwin-aarch64": f"Slop-Mail-{VERSION}-arm64.app.tar.gz",
         "windows-x86_64": f"Slop-Mail-{VERSION}-x64-setup.exe"}

def prepare(platform, bundle, output):
    output.mkdir(parents=True, exist_ok=True)
    package = output / NAMES[platform]
    if platform == "darwin-aarch64":
        with tarfile.open(package, "w:gz") as archive:
            archive.add(bundle / "macos/Slop Mail.app", arcname="Slop Mail.app")
        shutil.copyfile(bundle / f"dmg/Slop Mail_{VERSION}_aarch64.dmg",
                        output / f"Slop-Mail-{VERSION}-arm64.dmg")
    else:
        shutil.copyfile(bundle / f"nsis/Slop Mail_{VERSION}_x64-setup.exe", package)
    # Invoke the JS CLI directly so paths and signing inputs never pass through a shell.
    subprocess.run(["node", str(ROOT / "node_modules/@tauri-apps/cli/tauri.js"),
                    "signer", "sign", str(package)], check=True, stdout=subprocess.DEVNULL)
    if not Path(str(package) + ".sig").is_file():
        raise ValueError("Missing update signature")

def manifest(output, tag):
    if tag != f"v{VERSION}":
        raise ValueError("Release tag must match tauri.conf.json")
    platforms = {}
    for platform, name in NAMES.items():
        if not (output / name).is_file():
            raise ValueError(f"Missing {name}")
        platforms[platform] = {"signature": (output / (name+".sig")).read_text().strip(),
            "url": f"https://github.com/Lanakod/slop-mail/releases/download/{tag}/{name}"}
    (output / "latest.json").write_text(json.dumps({"version": VERSION, "platforms": platforms}, indent=2))

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=[*NAMES, "manifest"])
    parser.add_argument("--bundle", type=Path, default=ROOT / "target/release/bundle")
    parser.add_argument("--output", type=Path, default=ROOT / "target/release-assets")
    parser.add_argument("--tag")
    args = parser.parse_args()
    if args.action == "manifest":
        manifest(args.output, args.tag)
    else:
        prepare(args.action, args.bundle, args.output)
