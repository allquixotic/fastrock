#!/usr/bin/env python3
"""Sign/notarize Fastrock using the provisioned local Developer ID identity.

No private keys are exported. Run on a Mac with the identity and AC_NOTARY
profile installed. Reports are retained even if notarization fails or times out.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

IDENTITY = "9A3CFFC04D3472208A62C48E707EA6D4261998A1"
PROFILE = "AC_NOTARY"
REQUIREMENT = ('anchor apple generic and identifier "com.allquixotic.fastrock" '
               'and certificate leaf[subject.OU] = "B6XDYNLMPU" '
               'and certificate 1[field.1.2.840.113635.100.6.2.6] exists '
               'and certificate leaf[field.1.2.840.113635.100.6.1.13] exists')
PUBLISHER_REQUIREMENT = REQUIREMENT.replace(' and identifier "com.allquixotic.fastrock"', '')


def valid_signature(path, requirement):
    return subprocess.run(["codesign", "--verify", "--deep", "--strict", "-R", "=" + requirement, str(path)],
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0


def stapled(path):
    return subprocess.run(["xcrun", "stapler", "validate", str(path)],
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0


def run(*args, capture=False):
    return subprocess.run(args, check=True, text=True,
                          stdout=subprocess.PIPE if capture else None).stdout


def notarize(path, reports):
    report = reports / (path.name + ".submission.json")
    # Submit without waiting so the submission ID is durably recorded first.
    # Retrying after a timeout resumes the existing submission, not another upload.
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    data = json.loads(report.read_text()) if report.exists() else {}
    if data.get("sha256") != digest:
        data = json.loads(run("xcrun", "notarytool", "submit", str(path),
                              "--keychain-profile", PROFILE, "--output-format", "json", capture=True))
        data["sha256"] = digest
        report.write_text(json.dumps(data, indent=2) + "\n")
    submission = data["id"]
    result = json.loads(run("xcrun", "notarytool", "wait", submission,
                            "--keychain-profile", PROFILE, "--timeout", "10m",
                            "--output-format", "json", capture=True))
    (reports / (path.name + ".result.json")).write_text(json.dumps(result, indent=2) + "\n")
    log_path = reports / (path.name + ".notary-log.json")
    run("xcrun", "notarytool", "log", submission, "--keychain-profile", PROFILE, str(log_path))
    log = json.loads(log_path.read_text())
    if result.get("status") != "Accepted" or log.get("status") != "Accepted":
        raise RuntimeError(f"Notarization rejected {submission}; inspect {log_path}")
    print(f"Accepted {submission}; log: {log_path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    target = parser.add_mutually_exclusive_group(required=True)
    target.add_argument("--app", type=Path)
    target.add_argument("--dmg", type=Path)
    parser.add_argument("--reports", type=Path, required=True)
    args = parser.parse_args()
    args.reports.mkdir(parents=True, exist_ok=True)
    identities = run("security", "find-identity", "-v", "-p", "codesigning", capture=True)
    if IDENTITY not in identities:
        raise RuntimeError("The pinned Developer ID identity and private key are unavailable")
    run("xcrun", "notarytool", "history", "--keychain-profile", PROFILE,
        "--output-format", "json", capture=True)
    if args.app:
        app = args.app.resolve()
        executable = app / "Contents/MacOS/fastrock"
        # This app has one native executable, and no frameworks or helpers.
        # Refuse unexpected nested code rather than silently leaving it unsigned.
        for directory, dirs, files in os.walk(app, followlinks=False):
            for name in files:
                file = Path(directory) / name
                kind = run("file", "-b", str(file), capture=True)
                if "Mach-O" in kind and file != executable:
                    raise RuntimeError(f"Unexpected nested code needs a signing rule: {file}")
        valid = valid_signature(app, REQUIREMENT)
        for file in (() if valid else (executable, app)):
            run("codesign", "--force", "--timestamp", "--options", "runtime",
                "--sign", IDENTITY, str(file))
        run("codesign", "--verify", "--deep", "--strict", "-R", "=" + REQUIREMENT, str(app))
        if not stapled(app):
            archive = args.reports / "Fastrock-notary.zip"
            if archive.exists():
                archive.unlink()
            run("ditto", "-c", "-k", "--keepParent", str(app), str(archive))
            notarize(archive, args.reports)
            run("xcrun", "stapler", "staple", str(app))
        run("xcrun", "stapler", "validate", str(app))
        run("codesign", "--verify", "--deep", "--strict", "-R", "=" + REQUIREMENT, str(app))
        run("spctl", "--assess", "--type", "execute", "--verbose=4", str(app))
    else:
        dmg = args.dmg.resolve()
        if not valid_signature(dmg, PUBLISHER_REQUIREMENT):
            run("codesign", "--force", "--timestamp", "--sign", IDENTITY, str(dmg))
        run("codesign", "--verify", "--strict", str(dmg))
        if not stapled(dmg):
            notarize(dmg, args.reports)
            run("xcrun", "stapler", "staple", str(dmg))
        run("xcrun", "stapler", "validate", str(dmg))
        run("spctl", "--assess", "--type", "open", "--context", "context:primary-signature", "--verbose=4", str(dmg))
        # Verify the distributed app, not just the source bundle used to make
        # the image. Mount read-only and never launch the GUI during signing.
        with tempfile.TemporaryDirectory(prefix="fastrock-dmg-") as mount:
            run("hdiutil", "attach", str(dmg), "-readonly", "-nobrowse", "-mountpoint", mount)
            try:
                app = Path(mount) / "Fastrock.app"
                run("codesign", "--verify", "--deep", "--strict", "-R", "=" + REQUIREMENT, str(app))
                run("xcrun", "stapler", "validate", str(app))
                run("spctl", "--assess", "--type", "execute", "--verbose=4", str(app))
            finally:
                run("hdiutil", "detach", mount)


if __name__ == "__main__":
    main()
