"""Build RCode and print the executable reported by this Cargo invocation."""

import argparse
import json
from pathlib import Path
import subprocess
import sys


def build_executable(root, profile="release", target=None, cargo_command=None):
    command = list(cargo_command or ["cargo"])
    command += ["build", "--locked", "-p", "rcode", "--message-format=json-render-diagnostics"]
    if profile == "release":
        command.append("--release")
    elif profile != "dev":
        raise ValueError(f"Unsupported packaging profile: {profile}")
    if target:
        command += ["--target", target]
    executable = None
    process = subprocess.Popen(
        command, cwd=root, stdout=subprocess.PIPE, text=True, encoding="utf-8", errors="replace"
    )
    with process:
        for line in process.stdout:
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                print(line.rstrip(), file=sys.stderr)
                continue
            if message.get("reason") == "compiler-message":
                rendered = message.get("message", {}).get("rendered")
                if rendered:
                    print(rendered, file=sys.stderr, end="" if rendered.endswith("\n") else "\n")
            elif message.get("reason") == "compiler-artifact":
                built_target = message.get("target", {})
                if (built_target.get("name") == "rcode"
                        and "bin" in built_target.get("kind", [])
                        and message.get("executable")):
                    executable = Path(message["executable"])
        status = process.wait()
    if status:
        raise RuntimeError(f"Cargo packaging build failed with exit code {status}")
    if executable is None:
        raise RuntimeError("Cargo did not report an RCode executable; refusing a stale target fallback")
    if not executable.is_absolute():
        executable = Path(root) / executable
    if not executable.is_file():
        raise RuntimeError(f"Cargo reported a missing executable: {executable}")
    return executable.resolve()


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=("release", "dev"), default="release")
    parser.add_argument("--target")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    try:
        executable = build_executable(root, args.profile, args.target)
    except (OSError, RuntimeError) as error:
        parser.exit(1, f"{error}\n")
    print(executable)


if __name__ == "__main__":
    main()
