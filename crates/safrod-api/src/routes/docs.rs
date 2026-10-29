use axum::Router;
use axum::http::header;
use axum::response::{Html, IntoResponse};
use axum::routing::get;

use crate::AppState;

const OPENAPI: &str = include_str!("../../openapi.json");

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/openapi.json", get(openapi))
        .route("/docs", get(swagger))
        .route("/dashboard", get(dashboard))
}

async fn openapi() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/json")], OPENAPI)
}

/// Swagger UI is loaded from a pinned CDN release; the raw document at
/// `/openapi.json` works offline.
async fn swagger() -> Html<&'static str> {
    Html(
        r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Safrod API</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5.17.14/swagger-ui.css"></head>
<body><div id="ui"></div>
<noscript>Enable JavaScript or read <a href="/openapi.json">/openapi.json</a>.</noscript>
<script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5.17.14/swagger-ui-bundle.js"></script>
<script>SwaggerUIBundle({ url: "/openapi.json", dom_id: "#ui" });</script>
</body></html>"##,
    )
}

const DASHBOARD: &str = include_str!("../../../../dashboard/index.html");

/// Static page; it holds no secrets and calls the API with a key the operator
/// pastes in. The CSP keeps it from loading or contacting anything else.
async fn dashboard() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; base-uri 'none'; form-action 'none'",
            ),
        ],
        DASHBOARD,
    )
}
