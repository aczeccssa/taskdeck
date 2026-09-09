//! Auth middleware and login tests.

use std::collections::BTreeMap;
use std::path::PathBuf;

use axum::body::{Body, to_bytes};
use axum::http::Request as HttpRequest;
use serde_json::json;
use tower::util::ServiceExt;

use super::super::*;
use super::helpers::*;
use crate::config::{ProjectDefinition, TaskSpec};
use crate::runtime::SessionRuntime;

    #[tokio::test]

    pub(super) async fn auth_middleware_allows_disabled_and_protects_enabled_control_plane() {

        let state = DaemonState::new();

        let response = http_route(state.clone(), "GET", "/api/nodes", &[], None).await;

        assert_eq!(response.status(), StatusCode::OK);



        state.store.set_access_key("test-access-key").unwrap();

        state.store.configure_auth(true).unwrap();

        let unauthorized = http_route(state.clone(), "GET", "/api/nodes", &[], None).await;

        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let login = http_route(state, "GET", "/", &[], None).await;

        assert_eq!(login.status(), StatusCode::OK);

    }



    #[tokio::test]

    pub(super) async fn login_get_serves_the_react_shell() {

        let response = http_route(DaemonState::new(), "GET", "/login", &[], None).await;

        assert_eq!(response.status(), StatusCode::OK);

        assert_eq!(

            response.headers().get(header::CONTENT_TYPE).unwrap(),

            "text/html; charset=utf-8"

        );

    }



    #[tokio::test]

    pub(super) async fn auth_enabled_still_exposes_react_shell_and_hashed_assets() {

        let state = DaemonState::new();

        state.store.set_access_key("test-access-key").unwrap();

        state.store.configure_auth(true).unwrap();

        let shell = http_route(state.clone(), "GET", "/", &[], None).await;

        assert_eq!(shell.status(), StatusCode::OK);

        let asset = EMBEDDED_ASSETS

            .iter()

            .find(|asset| asset.path.starts_with("/assets/") && asset.path.ends_with(".js"))

            .unwrap();

        let static_asset = http_route(state, "GET", asset.path, &[], None).await;

        assert_eq!(static_asset.status(), StatusCode::OK);

        assert_eq!(

            static_asset.headers().get(header::CACHE_CONTROL).unwrap(),

            "public, max-age=31536000, immutable"

        );

    }



    #[tokio::test]

    pub(super) async fn auth_login_creates_a_session_cookie_accepted_by_api() {

        let state = DaemonState::new();

        state.store.set_access_key("test-access-key").unwrap();

        state.store.configure_auth(true).unwrap();

        let wrong = async_body_login(state.clone(), "bad").await;

        assert_eq!(wrong.status(), StatusCode::OK); // login page with error body

        let correct = async_body_login(state.clone(), "test-access-key").await;

        assert_eq!(correct.status(), StatusCode::SEE_OTHER);

        let cookie = correct

            .headers()

            .get(header::SET_COOKIE)

            .and_then(|v| v.to_str().ok())

            .map(str::to_string)

            .unwrap();

        let token = cookie

            .split(';')

            .next()

            .unwrap()

            .split('=')

            .nth(1)

            .unwrap()

            .to_string();

        let name = header::HeaderName::from_static("cookie");

        let cookie_value = format!("{AUTH_COOKIE}={token}");

        let response = http_route(

            state,

            "GET",

            "/api/nodes",

            &[(name, cookie_value.as_str())],

            None,

        )

        .await;

        assert_eq!(response.status(), StatusCode::OK);

    }
