use tempfile::tempdir;
use uuid::Uuid;

use super::{
    clear_session_at_path, effective_server, load_session_from_path, save_session_to_path, Session,
    DEFAULT_SERVER,
};

fn test_session() -> Session {
    Session {
        access_token: "access".into(),
        refresh_token: "refresh".into(),
        server: "http://localhost:8080".into(),
        username: "alice".into(),
        user_id: Uuid::new_v4(),
    }
}

#[test]
fn session_round_trips_to_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("session.json");
    let session = test_session();

    save_session_to_path(&session, &path).unwrap();
    let loaded = load_session_from_path(&path).unwrap();

    assert_eq!(loaded, session);
}

#[test]
fn clear_session_ignores_missing_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("missing.json");

    clear_session_at_path(&path).unwrap();
}

#[test]
fn effective_server_uses_session_when_cli_has_default() {
    let session = test_session();

    assert_eq!(effective_server(DEFAULT_SERVER, &session), session.server);
}

#[test]
fn effective_server_uses_explicit_cli_value() {
    let session = test_session();

    assert_eq!(
        effective_server("http://other-server:8080", &session),
        "http://other-server:8080"
    );
}
