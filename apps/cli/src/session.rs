//! Session management — JWT storage and refresh for the FreeBox CLI.

/// An active session with a FreeBox server.
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    pub server: String,
}
