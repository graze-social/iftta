#[cfg(test)]
mod tests {
    use super::super::*;
    use chrono::{Duration, Utc};
    use ulid::Ulid;

    #[test]
    fn test_session_type_conversion() {
        assert_eq!(SessionType::OAuth.as_str(), "oauth");
        assert_eq!(SessionType::AppPassword.as_str(), "app_password");

        assert_eq!(SessionType::from_string("oauth"), Some(SessionType::OAuth));
        assert_eq!(
            SessionType::from_string("app_password"),
            Some(SessionType::AppPassword)
        );
        assert_eq!(SessionType::from_string("invalid"), None);
    }

    #[test]
    fn test_session_type_default() {
        let default_type = SessionType::default();
        assert_eq!(default_type, SessionType::OAuth);
    }

    #[test]
    fn test_create_oauth_session() {
        let session_id = Ulid::new().to_string();
        let did = "did:plc:test123".to_string();
        let access_token = "access_token_123".to_string();
        let refresh_token = Some("refresh_token_456".to_string());
        let expires_at = Utc::now() + Duration::hours(1);

        let session = Session::new_oauth(
            session_id.clone(),
            did.clone(),
            access_token.clone(),
            refresh_token.clone(),
            expires_at,
        );

        assert_eq!(session.session_id, session_id);
        assert_eq!(session.did, did);
        assert_eq!(session.access_token, access_token);
        assert_eq!(session.refresh_token, refresh_token);
        assert_eq!(session.access_token_expires_at, expires_at);
        assert_eq!(session.session_type, SessionType::OAuth);
        assert!(session.is_oauth());
        assert!(!session.is_app_password());
    }

    #[test]
    fn test_create_app_password_session() {
        let session_id = Ulid::new().to_string();
        let did = "did:plc:test123".to_string();
        let access_token = "app_password_token_789".to_string();
        let expires_at = Utc::now() + Duration::days(30);

        let session = Session::new_app_password(
            session_id.clone(),
            did.clone(),
            access_token.clone(),
            expires_at,
        );

        assert_eq!(session.session_id, session_id);
        assert_eq!(session.did, did);
        assert_eq!(session.access_token, access_token);
        assert_eq!(session.refresh_token, None);
        assert_eq!(session.access_token_expires_at, expires_at);
        assert_eq!(session.session_type, SessionType::AppPassword);
        assert!(!session.is_oauth());
        assert!(session.is_app_password());
    }
}
