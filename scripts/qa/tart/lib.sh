# shellcheck shell=bash
# Shared settings and helpers for the Tart QA scripts. Source it; do not run it.
#
# Everything lives under TART_QA_WORK (VM images, keys, logs). Point it at a disk with room:
# the base image takes about 25-30 GB; a clone costs little until its guest writes.

: "${TART_QA_WORK:?set TART_QA_WORK to a work directory on a disk with at least 60 GB free}"
export TART_HOME="${TART_HOME:-$TART_QA_WORK/tart-home}"
export TMPDIR="$TART_HOME/tmp/"
# Tart otherwise prunes its caches on its own when the disk is tight; keep every deletion explicit.
export TART_NO_AUTO_PRUNE=1
TART="${TART:-tart}"
TART_QA_BASE="${TART_QA_BASE:-corbanu-qa-base}"
TART_QA_KEY="${TART_QA_KEY:-$TART_QA_WORK/keys/tart-qa_ed25519}"
TART_QA_KNOWN_HOSTS="${TART_QA_KNOWN_HOSTS:-$TART_QA_WORK/keys/known_hosts}"
UTMCTL="${UTMCTL:-/Applications/UTM.app/Contents/MacOS/utmctl}"
LOGS="$TART_QA_WORK/logs"
mkdir -p "$TMPDIR" "$LOGS"

log() { printf '%s %s\n' "$(date -u +%FT%TZ)" "$*" >&2; }
die() { log "ERROR: $*"; exit 1; }
now() { python3 -c 'import time; print(f"{time.time():.3f}")'; }

# Running macOS guests on this host. macOS allows two at once, shared by every tool (UTM, Tart, ...).
running_tart_vms() {
  "$TART" list --source local --format json |
    python3 -c 'import json,sys; print(sum(1 for v in json.load(sys.stdin) if v.get("Running") or v.get("State") == "running"))'
}
running_utm_vms() {
  if [ -x "$UTMCTL" ]; then "$UTMCTL" list 2>/dev/null | awk 'NR > 1 && $2 == "started"' | wc -l | tr -d ' '; else echo 0; fi
}

# Refuse to start a VM unless it would be the only Tart VM and a macOS slot is free. Never touches other VMs.
guard_slot() {
  local tart_running utm_running
  tart_running="$(running_tart_vms)"
  utm_running="$(running_utm_vms)"
  [ "$tart_running" = 0 ] || die "a Tart VM is already running; these scripts run one at a time"
  [ $((tart_running + utm_running)) -lt 2 ] || die "two macOS guests already running (UTM: $utm_running); no free slot"
  log "slot ok (UTM running: $utm_running, Tart running: $tart_running)"
}

SSH_OPTS=(-i "$TART_QA_KEY" -o IdentitiesOnly=yes -o UserKnownHostsFile="$TART_QA_KNOWN_HOSTS"
  -o StrictHostKeyChecking=yes -o HostKeyAlias=tart-qa-base -o BatchMode=yes -o ConnectTimeout=5
  -o ForwardAgent=no -o ClearAllForwardings=yes -o LogLevel=ERROR)
# Every clone of the base shares its SSH host key, which prepare-base.sh pins under the alias above.
# shellcheck disable=SC2029  # remote commands are built on purpose
gssh() { local ip="$1"; shift; ssh "${SSH_OPTS[@]}" "admin@$ip" "$@"; }

start_vm() { # NAME: run the VM headless in the background, without host shares, audio or clipboard
  local name="$1"
  guard_slot
  nohup "$TART" run --no-graphics --no-audio --no-clipboard "$name" > "$LOGS/$name.run.log" 2>&1 &
  echo $! > "$LOGS/$name.pid"
}

wait_ip() { "$TART" ip --wait "${2:-180}" "$1"; }

wait_ssh() { # IP [SECONDS]
  local ip="$1" limit="${2:-180}" start
  start=$(date +%s)
  until gssh "$ip" true 2>/dev/null; do
    [ $(($(date +%s) - start)) -lt "$limit" ] || return 1
    sleep 1
  done
}

stop_vm() { # NAME [TIMEOUT]: shutdown request, power-off after TIMEOUT seconds; waits for `tart run` to exit
  local name="$1" pid
  "$TART" stop --timeout "${2:-30}" "$name" > /dev/null 2>&1 || true
  if [ -f "$LOGS/$name.pid" ]; then
    pid="$(cat "$LOGS/$name.pid")"
    while kill -0 "$pid" 2> /dev/null; do sleep 0.5; done
    rm "$LOGS/$name.pid"
  fi
}

# PIDs of Virtualization.framework VM processes (one per running guest, UTM's included).
vm_pids() { pgrep -f com.apple.Virtualization.VirtualMachine | sort; }
