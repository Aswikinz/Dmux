//! CLI-level tests for `--ca-file` and the `ca_file` definition key (audit M9
//! follow-up).  They drive the real `dmux` binary so they observe what a user
//! sees.  The CA file is read before any network traffic, so the error cases
//! need no server; the live case runs only when `DMUX_TEST_TLS_URL` and
//! `DMUX_TEST_TLS_CA` are set (the TLS-only test container).

use std::path::PathBuf;
use std::process::{Command, Output};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dmux-cli-tls-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `dmux probe` against a closed local port, with the given CA file.
fn probe_with_ca(ca: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dmux"))
        .args([
            "probe",
            "--host",
            "127.0.0.1",
            "--port",
            "1",
            "--db",
            "d",
            "--user",
            "u",
            "--ca-file",
            ca,
        ])
        .output()
        .expect("run dmux probe")
}

#[test]
fn a_missing_ca_file_is_a_clear_error_naming_the_path() {
    let dir = temp_dir("missing");
    let ca = dir.join("no-such-ca.pem");
    let out = probe_with_ca(ca.to_str().unwrap());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(
        stderr.contains("could not read CA file") && stderr.contains(ca.to_str().unwrap()),
        "the error must name the CA file: {stderr}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_invalid_pem_is_a_clear_error_that_never_echoes_the_contents() {
    let dir = temp_dir("invalid");
    let sentinel = format!("NOT-A-CERT-{}", std::process::id());
    // Not PEM at all, and a PEM block whose body is not base64.
    let cases = [
        ("plain.pem", sentinel.clone()),
        (
            "broken.pem",
            format!("-----BEGIN CERTIFICATE-----\n{sentinel}!!\n-----END CERTIFICATE-----\n"),
        ),
    ];
    for (name, body) in cases {
        let ca = dir.join(name);
        std::fs::write(&ca, &body).unwrap();
        let out = probe_with_ca(ca.to_str().unwrap());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{name}");
        assert!(
            stderr.contains(&format!("CA file {}", ca.display())),
            "the error must name the CA file ({name}): {stderr}"
        );
        assert!(
            stderr.contains("no certificates") || stderr.contains("not a valid PEM"),
            "the error must say what is wrong ({name}): {stderr}"
        );
        assert!(
            !stderr.contains(&sentinel),
            "the file's contents must never be echoed ({name}): {stderr}"
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn ca_file_with_tls_disable_is_refused_before_connecting() {
    let out = Command::new(env!("CARGO_BIN_EXE_dmux"))
        .args([
            "probe",
            "--host",
            "127.0.0.1",
            "--port",
            "1",
            "--db",
            "d",
            "--user",
            "u",
            "--tls",
            "disable",
            "--reason",
            "lab",
            "--ca-file",
            "/ca.pem",
        ])
        .output()
        .expect("run dmux probe");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(
        stderr.contains("--ca-file cannot be used with --tls disable"),
        "{stderr}"
    );
}

/// `postgres://user:pass@host:port/db` into its parts.
fn parse_url(url: &str) -> (String, String, String, String, String) {
    let rest = url.strip_prefix("postgres://").expect("postgres:// URL");
    let (auth, hostdb) = rest.split_once('@').expect("user@host");
    let (user, pass) = auth.split_once(':').expect("user:pass");
    let (hostport, db) = hostdb.split_once('/').expect("host/db");
    let (host, port) = hostport.split_once(':').expect("host:port");
    (
        user.to_string(),
        pass.to_string(),
        host.to_string(),
        port.to_string(),
        db.to_string(),
    )
}

#[test]
fn live_ca_file_connects_with_probe_and_run() {
    let (Ok(url), Ok(ca)) = (
        std::env::var("DMUX_TEST_TLS_URL"),
        std::env::var("DMUX_TEST_TLS_CA"),
    ) else {
        eprintln!("note: DMUX_TEST_TLS_URL / DMUX_TEST_TLS_CA not set; skipping the live CA test");
        return;
    };
    let (user, pass, host, port, db) = parse_url(&url);
    const PW_VAR: &str = "DMUX_TEST_TLS_CLI_PW";

    // probe --ca-file: verified TLS against the private CA.
    let out = Command::new(env!("CARGO_BIN_EXE_dmux"))
        .env(PW_VAR, &pass)
        .args([
            "probe",
            "--host",
            &host,
            "--port",
            &port,
            "--db",
            &db,
            "--user",
            &user,
            "--password-env",
            PW_VAR,
            "--ca-file",
            &ca,
            "--allow-superuser",
            "--reason",
            "local test container",
        ])
        .output()
        .expect("run dmux probe");
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The test login is a superuser that can write, so the probe exits with the
    // warning code; what matters is that it connected and reported the server.
    assert!(
        !stderr.contains("probe failed"),
        "probe with --ca-file must connect: {stderr}"
    );
    let info: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("probe must print the server info ({e}): {stderr}"));
    assert!(info["version_num"].as_i64().unwrap_or(0) > 0, "{info}");

    // Without the CA file, verify-full must not connect.
    let out = Command::new(env!("CARGO_BIN_EXE_dmux"))
        .env(PW_VAR, &pass)
        .args([
            "probe",
            "--host",
            &host,
            "--port",
            &port,
            "--db",
            &db,
            "--user",
            &user,
            "--password-env",
            PW_VAR,
            "--allow-superuser",
            "--reason",
            "local test container",
        ])
        .output()
        .expect("run dmux probe");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("probe failed"),
        "a private CA must not verify against the public roots"
    );

    // dmux run with ca_file in the definition: a structure run over verified TLS.
    let dir = temp_dir("live");
    let quote = |s: &str| serde_json::to_string(s).unwrap();
    let conn = format!(
        "{{ host = {}, port = {port}, db = {}, user = {}, password_env = {}, ca_file = {} }}",
        quote(&host),
        quote(&db),
        quote(&user),
        quote(PW_VAR),
        quote(&ca)
    );
    let def = dir.join("tls.toml");
    std::fs::write(
        &def,
        format!(
            "level = \"structure\"\nallow_superuser = true\nreason = \"local test container\"\nold = {conn}\nnew = {conn}\n"
        ),
    )
    .unwrap();
    let results = dir.join("results.json");
    let out = Command::new(env!("CARGO_BIN_EXE_dmux"))
        .env(PW_VAR, &pass)
        .args([
            "run",
            def.to_str().unwrap(),
            "--out",
            results.to_str().unwrap(),
        ])
        .output()
        .expect("run dmux run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        results.exists(),
        "dmux run with ca_file must complete: {stderr}"
    );
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&results).unwrap()).unwrap();
    for side in ["old", "new"] {
        assert_eq!(
            json["connections"][side]["tls"], "verified",
            "{side} side must be verified TLS"
        );
    }
    assert_ne!(json["verdict"], "TOOL_FAULT", "{stderr}");
    std::fs::remove_dir_all(&dir).unwrap();
}
