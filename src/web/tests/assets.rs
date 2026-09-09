//! Embedded asset tests.

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

    #[test]

    pub(super) fn embedded_frontend_has_root_document_and_hashed_assets() {

        assert!(

            EMBEDDED_ASSETS

                .iter()

                .any(|asset| asset.path == "/index.html")

        );

        assert!(

            EMBEDDED_ASSETS

                .iter()

                .any(|asset| asset.path.starts_with("/assets/") && asset.path.ends_with(".js"))

        );

        assert!(

            EMBEDDED_ASSETS

                .iter()

                .any(|asset| asset.path.starts_with("/assets/") && asset.path.ends_with(".css"))

        );

        assert!(

            EMBEDDED_ASSETS

                .iter()

                .any(|asset| asset.path == "/favicon.svg")

        );

    }



    #[test]

    pub(super) fn embedded_asset_responses_use_expected_cache_headers() {

        let html = embedded_asset_response("/index.html", false);

        assert_eq!(

            html.headers().get(header::CONTENT_TYPE).unwrap(),

            "text/html; charset=utf-8"

        );

        assert_eq!(

            html.headers().get(header::CACHE_CONTROL).unwrap(),

            "no-cache"

        );

        let asset = EMBEDDED_ASSETS

            .iter()

            .find(|asset| asset.path.starts_with("/assets/") && asset.path.ends_with(".js"))

            .unwrap();

        let response = embedded_asset_response(asset.path, true);

        assert_eq!(

            response.headers().get(header::CONTENT_TYPE).unwrap(),

            "application/javascript; charset=utf-8"

        );

        assert_eq!(

            response.headers().get(header::CACHE_CONTROL).unwrap(),

            "public, max-age=31536000, immutable"

        );

    }
