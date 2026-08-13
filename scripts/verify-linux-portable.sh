#!/usr/bin/env bash
# Run a musl Linux release binary on AlmaLinux 8 (GLIBC 2.28) and exercise
# --version plus a PostgreSQL-backed `list`. No Rust/Cargo on the guest.
set -euo pipefail

binary="${1:-}"
if [[ -z "$binary" || ! -f "$binary" ]]; then
  echo "usage: $0 <path-to-conversation-handoff-linux-x86_64>" >&2
  exit 2
fi
binary="$(cd "$(dirname "$binary")" && pwd)/$(basename "$binary")"

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required to verify the binary on AlmaLinux." >&2
  exit 1
fi

echo "== file =="
file "$binary"
info="$(file "$binary")"
if echo "$info" | grep -qi 'dynamically linked'; then
  echo "ERROR: binary is dynamically linked; it will depend on the host libc." >&2
  exit 1
fi
if ! echo "$info" | grep -Eqi 'statically linked|static-pie linked'; then
  echo "ERROR: expected a statically linked musl binary." >&2
  exit 1
fi

echo "== ldd (must not resolve GLIBC) =="
ldd_out="$(mktemp)"
ldd "$binary" >"$ldd_out" 2>&1 || true
cat "$ldd_out"
if grep -qi glibc "$ldd_out"; then
  echo "ERROR: ldd mentioned GLIBC." >&2
  exit 1
fi
if grep -E '\.so' "$ldd_out" >/dev/null; then
  echo "ERROR: binary links shared libraries:" >&2
  cat "$ldd_out" >&2
  exit 1
fi
if ! grep -Eqi 'statically linked|not a dynamic executable' "$ldd_out"; then
  echo "ERROR: ldd did not report a static binary." >&2
  exit 1
fi
rm -f "$ldd_out"

network="ch-portability-$$"
pg="ch-pg-$$"
guest="almalinux:8"
cleanup() {
  docker rm -f "$pg" >/dev/null 2>&1 || true
  docker network rm "$network" >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "== AlmaLinux 8 --version =="
docker pull -q "$guest"
docker run --rm -v "$binary:/opt/conversation-handoff:ro" "$guest" \
  /opt/conversation-handoff --version

echo "== PostgreSQL-backed list on AlmaLinux 8 =="
docker network create "$network" >/dev/null
docker run -d --name "$pg" --network "$network" \
  -e POSTGRES_PASSWORD=pass \
  -e POSTGRES_USER=handoff \
  -e POSTGRES_DB=handoff \
  postgres:16 >/dev/null
for _ in $(seq 1 40); do
  if docker exec "$pg" pg_isready -U handoff >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
docker exec "$pg" pg_isready -U handoff >/dev/null

cfg="$(mktemp)"
cat >"$cfg" <<EOF
store:
  type: postgres
  url: "$pg:5432/handoff"
  user: handoff
  password: pass
  ssl: false
EOF

docker run --rm --network "$network" \
  -v "$binary:/opt/conversation-handoff:ro" \
  -v "$cfg:/config.yaml:ro" \
  -e CONVERSATION_HANDOFF_CONFIG=/config.yaml \
  "$guest" \
  /opt/conversation-handoff list

rm -f "$cfg"
echo "OK: binary runs on AlmaLinux 8 without GLIBC, Rust, or Cargo."
