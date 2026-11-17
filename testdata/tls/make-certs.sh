#!/usr/bin/env bash
# tls: generate the throwaway certificates for the TLS-only mock (T052).
#
# Usage:
#   testdata/tls/make-certs.sh
#
# Writes into testdata/tls/out/ (git-ignored):
#   ca.crt, ca.key                     a private test CA
#   server.crt, server.key             a server certificate for 127.0.0.1 and
#                                      localhost, signed by the CA (used by the
#                                      `tls` compose service; verify-full trust)
#   selfsigned.crt, selfsigned.key     a self-signed leaf with the same SANs,
#                                      for the trust-once (pin-fingerprint) test
#
# These are for the throwaway local mock only: they are generated at run time
# and never committed. Regenerating is safe - out/ is recreated and the compose
# service bind-mounts it, so `podman compose --profile tls up -d` picks up the
# new files.
#
# The private keys are chmod 600 and chown'd to the container's postgres user
# (uid 999) via `podman unshare`, because the postgres image runs the server as
# uid 999 and refuses a key with group/world access or the wrong owner. The
# certificates stay world-readable (they are public).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="$HERE/out"

command -v openssl >/dev/null 2>&1 || { echo "make-certs.sh: openssl is required" >&2; exit 1; }
command -v podman >/dev/null 2>&1 || { echo "make-certs.sh: podman is required" >&2; exit 1; }

rm -rf "$OUT"
mkdir -p "$OUT"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Private test CA.
openssl req -new -x509 -days 3650 -nodes -newkey rsa:2048 \
  -keyout "$OUT/ca.key" -out "$OUT/ca.crt" \
  -subj "/CN=Dmux test CA" \
  -addext "basicConstraints=critical,CA:TRUE" \
  -addext "keyUsage=critical,keyCertSign,cRLSign" \
  -addext "subjectKeyIdentifier=hash" >/dev/null 2>&1

# Server key + CSR, signed by the CA with SANs for 127.0.0.1 and localhost.
openssl req -new -nodes -newkey rsa:2048 \
  -keyout "$OUT/server.key" -out "$TMP/server.csr" \
  -subj "/CN=localhost" >/dev/null 2>&1
cat > "$TMP/server.ext" <<'EOF'
subjectAltName=DNS:localhost,IP:127.0.0.1
keyUsage=digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
EOF
printf '1000\n' > "$TMP/ca.srl"
openssl x509 -req -days 3650 \
  -in "$TMP/server.csr" -CA "$OUT/ca.crt" -CAkey "$OUT/ca.key" -CAserial "$TMP/ca.srl" \
  -out "$OUT/server.crt" -extfile "$TMP/server.ext" >/dev/null 2>&1

# Self-signed leaf with the same SANs, for the trust-once test.
openssl req -new -x509 -days 3650 -nodes -newkey rsa:2048 \
  -keyout "$OUT/selfsigned.key" -out "$OUT/selfsigned.crt" \
  -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" \
  -addext "keyUsage=digitalSignature,keyEncipherment" \
  -addext "extendedKeyUsage=serverAuth" >/dev/null 2>&1

# Certificates are public; keys are not. The postgres container runs as uid 999,
# so the keys must be owned by it (and 0600, or postgres refuses to load them).
chmod 644 "$OUT/ca.crt" "$OUT/server.crt" "$OUT/selfsigned.crt"
chmod 600 "$OUT/ca.key" "$OUT/server.key" "$OUT/selfsigned.key"
podman unshare chown 999:999 "$OUT/ca.key" "$OUT/server.key" "$OUT/selfsigned.key"

# Fail loudly if the server certificate does not verify against the CA.
openssl verify -CAfile "$OUT/ca.crt" "$OUT/server.crt"

echo "Wrote test certificates to $OUT/:"
find "$OUT" -maxdepth 1 -type f -printf '  %m %s  %f\n' | sort
echo
echo "Next: podman compose -f testdata/compose.yml --profile tls up -d"
