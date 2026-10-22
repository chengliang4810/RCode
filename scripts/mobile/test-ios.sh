#!/usr/bin/env bash
# macOS only. Run the shipping Flutter app against a local, provider-free
# EngineCore fixture on a new simulator. No signing or App Store credentials.
# Usage: bash scripts/mobile/test-ios.sh
set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
  echo 'The iOS device gate requires macOS, Xcode and an installed iOS simulator runtime.' >&2
  exit 2
fi
for tool in xcrun xcodebuild python3 cargo rustc flutter git; do
  command -v "$tool" >/dev/null || { echo "Required command not found: $tool" >&2; exit 2; }
done

task_root="$(cd "$(dirname "$0")/../.." && pwd)"
mobile_root="$task_root/apps/mobile"
artifact_parent="${RCODE_MOBILE_QA_ARTIFACTS:-$mobile_root/build/qa/ios}"
artifact_parent="$(python3 -c 'import os,sys; print(os.path.abspath(sys.argv[1]))' "$artifact_parent")"
artifact_dir="$artifact_parent/run-$(date +%Y%m%d-%H%M%S)-$$"
mkdir -p "$artifact_dir"
export RCODE_MOBILE_QA_ARTIFACTS="$artifact_dir" CARGO_INCREMENTAL=0
exec > >(tee "$artifact_dir/gate.log") 2>&1

simulator_udid=''
fixture_pid=''
bounded_pid=''

# macOS does not ship GNU timeout. Each bounded command owns a new process
# group; cancellation terminates that group, never an existing developer tool.
run_bounded() {
  python3 - "$@" <<'PY' &
import os
import signal
import subprocess
import sys

seconds = int(sys.argv[1])
if seconds <= 0:
    raise SystemExit("Command timeout must be positive")
command = subprocess.Popen(sys.argv[2:], stdin=subprocess.DEVNULL, start_new_session=True)

def interrupt(signum, _frame):
    raise SystemExit(128 + signum)

def stop(signum):
    try:
        os.killpg(command.pid, signum)
    except ProcessLookupError:
        pass

signal.signal(signal.SIGINT, interrupt)
signal.signal(signal.SIGTERM, interrupt)
try:
    try:
        result = command.wait(timeout=seconds)
    except subprocess.TimeoutExpired:
        print(f"Command timed out after {seconds}s: {sys.argv[2]}", file=sys.stderr, flush=True)
        result = 124
finally:
    if command.poll() is None:
        stop(signal.SIGINT)
        try:
            command.wait(timeout=10)
        except subprocess.TimeoutExpired:
            stop(signal.SIGTERM)
            try:
                command.wait(timeout=5)
            except subprocess.TimeoutExpired:
                stop(signal.SIGKILL)
                command.wait()
raise SystemExit(result if result >= 0 else 128 - result)
PY
  bounded_pid=$!
  local result=0
  wait "$bounded_pid" || result=$?
  bounded_pid=''
  return "$result"
}

cleanup() {
  local result=$? cleanup_failed=0 fixture_stopped=true simulator_deleted=true
  trap - EXIT INT TERM
  set +e
  if [[ -n "$bounded_pid" ]]; then
    kill -TERM "$bounded_pid" 2>/dev/null
    wait "$bounded_pid" 2>/dev/null
    bounded_pid=''
  fi
  if [[ -n "$simulator_udid" ]]; then
    if [[ "$result" != 0 ]]; then
      run_bounded 20 xcrun simctl io "$simulator_udid" screenshot \
        "$artifact_dir/failure-simulator.png" >>"$artifact_dir/cleanup.log" 2>&1
    fi
    run_bounded 30 xcrun simctl spawn "$simulator_udid" log show --last 10m \
      --style compact --predicate 'process == "Runner"' >"$artifact_dir/simulator.log" 2>&1
  fi
  if [[ -n "$fixture_pid" ]]; then
    # This PID is the binary this script started, not a process-name match.
    if kill -0 "$fixture_pid" 2>/dev/null; then
      kill -INT "$fixture_pid" 2>/dev/null
      for _ in {1..40}; do
        kill -0 "$fixture_pid" 2>/dev/null || break
        sleep 0.25
      done
      if kill -0 "$fixture_pid" 2>/dev/null; then
        kill -TERM "$fixture_pid" 2>/dev/null
        sleep 1
      fi
      if kill -0 "$fixture_pid" 2>/dev/null; then kill -KILL "$fixture_pid" 2>/dev/null; fi
    fi
    wait "$fixture_pid" 2>/dev/null
    if kill -0 "$fixture_pid" 2>/dev/null; then fixture_stopped=false; cleanup_failed=1; fi
  fi
  if [[ -n "$simulator_udid" ]]; then
    run_bounded 60 xcrun simctl shutdown "$simulator_udid" >>"$artifact_dir/cleanup.log" 2>&1
    if ! run_bounded 60 xcrun simctl delete "$simulator_udid" >>"$artifact_dir/cleanup.log" 2>&1; then
      simulator_deleted=false
      cleanup_failed=1
    fi
  fi
  if [[ "$result" == 0 && "$cleanup_failed" != 0 ]]; then result=1; fi
  printf '{"exitCode":%s,"fixtureStopped":%s,"simulatorDeleted":%s}\n' \
    "$result" "$fixture_stopped" "$simulator_deleted" >"$artifact_dir/cleanup.json"
  echo "iOS device gate exit=$result; artifacts: $artifact_dir"
  exit "$result"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

echo "iOS device gate artifacts: $artifact_dir"
git -C "$task_root" rev-parse HEAD >"$artifact_dir/source-revision.txt"
xcodebuild -version >"$artifact_dir/xcode.txt"
flutter --version >"$artifact_dir/flutter.txt"
rustc -Vv >"$artifact_dir/rustc.txt"
xcrun --sdk iphonesimulator --show-sdk-path >"$artifact_dir/simulator-sdk.txt"
xcrun simctl list runtimes --json >"$artifact_dir/runtimes.json"
xcrun simctl list devicetypes --json >"$artifact_dir/device-types.json"

# Use a compatible iPhone type from the newest available iOS runtime. Prefer
# the runtime's own supported-device list; older simctl versions expose bounds.
python3 - "$artifact_dir" <<'PY'
import json
import pathlib
import re
import sys

directory = pathlib.Path(sys.argv[1])
runtimes = json.loads((directory / "runtimes.json").read_text())["runtimes"]
types = json.loads((directory / "device-types.json").read_text())["devicetypes"]

def version(value):
    return tuple(int(part) for part in re.findall(r"\d+", value))

available = [runtime for runtime in runtimes
             if runtime.get("isAvailable", False)
             and runtime["identifier"].startswith("com.apple.CoreSimulator.SimRuntime.iOS-")]
for runtime in sorted(available, key=lambda item: version(item["version"]), reverse=True):
    supported = {item["identifier"] for item in runtime.get("supportedDeviceTypes", [])}
    parts = (version(runtime["version"]) + (0, 0))[:3]
    encoded_version = (parts[0] << 16) | (parts[1] << 8) | parts[2]
    candidates = [device for device in types
                  if (device.get("productFamily") == "iPhone" or device["name"].startswith("iPhone"))
                  and (device["identifier"] in supported if supported else
                       device.get("minRuntimeVersion", 0) <= encoded_version <=
                       device.get("maxRuntimeVersion", 0xffffffff))]
    if candidates:
        device = sorted(candidates, key=lambda item: item["name"])[0]
        selection = {"runtime": runtime["identifier"], "runtimeVersion": runtime["version"],
                     "deviceType": device["identifier"], "deviceName": device["name"]}
        (directory / "simulator-selection.json").write_text(json.dumps(selection, indent=2) + "\n")
        (directory / "runtime-id.txt").write_text(selection["runtime"] + "\n")
        (directory / "device-type-id.txt").write_text(selection["deviceType"] + "\n")
        break
else:
    raise SystemExit("No available iOS runtime with a compatible iPhone simulator type")
PY

cd "$task_root"
run_bounded "${RCODE_MOBILE_QA_BUILD_TIMEOUT_SECONDS:-1200}" \
  cargo build --locked -p rcode-engine --example mobile_smoke \
    --config 'profile.dev.package.fff-search.opt-level=0' > >(tee "$artifact_dir/fixture-build.log") 2>&1
target_dir="$(cargo metadata --locked --no-deps --format-version=1 | \
  python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
fixture_binary="$target_dir/debug/examples/mobile_smoke"
test -x "$fixture_binary"

runtime_id="$(cat "$artifact_dir/runtime-id.txt")"
device_type_id="$(cat "$artifact_dir/device-type-id.txt")"
simulator_udid="$(xcrun simctl create "RCode-QA-$(date +%s)-$$" "$device_type_id" "$runtime_id")"
if [[ ! "$simulator_udid" =~ ^[[:xdigit:]]{8}-[[:xdigit:]]{4}-[[:xdigit:]]{4}-[[:xdigit:]]{4}-[[:xdigit:]]{12}$ ]]; then
  echo 'simctl create did not return a simulator UDID.' >&2
  simulator_udid=''
  exit 1
fi
printf '%s\n' "$simulator_udid" >"$artifact_dir/simulator-udid.txt"
run_bounded 180 xcrun simctl bootstatus "$simulator_udid" -b > >(tee "$artifact_dir/simulator-boot.log") 2>&1

"$fixture_binary" >"$artifact_dir/fixture.log" 2>&1 &
fixture_pid=$!
printf '%s\n' "$fixture_pid" >"$artifact_dir/fixture-pid.txt"
ready_deadline=$((SECONDS + 90))
while ! grep -Fq 'Mobile QA mock ready (no provider requests)' "$artifact_dir/fixture.log"; do
  if ! kill -0 "$fixture_pid" 2>/dev/null || (( SECONDS >= ready_deadline )); then
    echo 'Engine fixture failed to reach its authenticated model-catalog readiness probe.' >&2
    tail -n 80 "$artifact_dir/fixture.log"
    exit 1
  fi
  sleep 0.25
done
kill -0 "$fixture_pid"

cd "$mobile_root"
run_bounded "${RCODE_MOBILE_QA_DRIVE_TIMEOUT_SECONDS:-900}" \
  flutter drive --debug --no-pub -d "$simulator_udid" \
    --driver=test_driver/integration_test.dart \
    --target=integration_test/remote_engine_test.dart \
    --screenshot="$artifact_dir" \
    --dart-define=RCODE_MOBILE_QA_URL=ws://127.0.0.1:27657 \
  > >(tee "$artifact_dir/flutter-drive.log") 2>&1
kill -0 "$fixture_pid"

python3 - "$artifact_dir" <<'PY'
import json
import pathlib
import re
import struct
import sys

directory = pathlib.Path(sys.argv[1])
report = json.loads((directory / "result.json").read_text())
checks = report.get("checks")
if (report.get("status") != "passed" or report.get("platform") != "ios"
        or not isinstance(checks, list) or len(checks) != 6):
    raise SystemExit("The device driver did not report all six passed iOS checks")
screenshots = report.get("screenshots", [])
required = {"01-authenticated-workspace", "02-native-renderers", "03-native-control",
            "04-accepted-slow-turn", "05-restored-offline-turn"}
if not isinstance(screenshots, list) or len(screenshots) != 5 or set(screenshots) != required:
    raise SystemExit("The device gate must produce all five requested screenshots")
for name in screenshots:
    if not isinstance(name, str) or not re.fullmatch(r"[a-zA-Z0-9_-]+", name):
        raise SystemExit("Invalid screenshot name in device report")
    image = directory / f"{name}.png"
    data = image.read_bytes()
    if (len(data) < 33 or not data.startswith(b"\x89PNG\r\n\x1a\n")
            or data[12:16] != b"IHDR" or not all(struct.unpack(">II", data[16:24]))):
        raise SystemExit(f"Missing or invalid PNG: {image.name}")
print(f"PASS: iOS runtime checks and {len(screenshots)} captured PNGs")
PY
