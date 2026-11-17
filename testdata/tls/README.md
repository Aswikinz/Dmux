# TLS fixture (`dmux-tls`, port 5439)

The fixture for the M4/M9 findings (SECURITY-AUDIT.md): a TLS-only Postgres 17
server on the mock set, so the engine's TLS paths — verified CA trust and the
trust-once self-signed flow — can be tested against a real encrypted endpoint.
It lives behind the `tls` compose profile, so the default `podman compose up`
is unchanged.

## Bring it up

```
testdata/tls/make-certs.sh
podman compose -f testdata/compose.yml --profile tls up -d
```

`make-certs.sh` writes a throwaway CA and server certificate into
`testdata/tls/out/` (git-ignored). The service bind-mounts that directory and a
committed hostssl-only `pg_hba.conf`, and starts Postgres with `ssl=on`.

## Connection

```
DMUX_TEST_TLS_URL=postgres://dmux_dev:dmux_dev_only@127.0.0.1:5439/bank
DMUX_TEST_TLS_CA=<checkout>/testdata/tls/out/ca.crt
```

* `sslmode=verify-full sslrootcert=$DMUX_TEST_TLS_CA` connects: `server.crt` is
  signed by the CA and carries `subjectAltName=DNS:localhost,IP:127.0.0.1`.
* plaintext (`sslmode=disable`) is refused (`no pg_hba.conf entry … no
  encryption`), because the `pg_hba.conf` has only `hostssl` entries for TCP.

## Files

| File | Purpose |
|---|---|
| `make-certs.sh` | generates `out/` (CA, server certificate, self-signed leaf) |
| `pg_hba.conf` | hostssl-only TCP; `local … trust` keeps the image's init working |
| `out/` | generated, git-ignored |

`out/selfsigned.crt` / `out/selfsigned.key` is a second, self-signed leaf with
the same SANs, for the trust-once (pin-the-fingerprint) test: a client that has
not been told to trust it must surface the untrusted certificate — not fall back
to plaintext. `out/ca.crt` is what the verified path trusts.
