use axum::http::header;
use axum::response::{Html, IntoResponse, Response};

const INDEX_HTML: &str = include_str!("../../web/index.html");
const APP_JS: &str = include_str!("../../web/app.js");
const STYLE_CSS: &str = include_str!("../../web/style.css");
const FAVICON_SVG: &str = include_str!("../../web/favicon.svg");
const AYAME_LOGO_SVG: &str = include_str!("../../web/ayame-logo.svg");
const IRIS_WATERCOLOR: &[u8] = include_bytes!("../../web/iris-watercolor.png");

pub(super) async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

pub(super) async fn app_js() -> Response {
    asset("application/javascript; charset=utf-8", APP_JS)
}

pub(super) async fn style_css() -> Response {
    asset("text/css; charset=utf-8", STYLE_CSS)
}

pub(super) async fn favicon_svg() -> Response {
    asset("image/svg+xml; charset=utf-8", FAVICON_SVG)
}

pub(super) async fn ayame_logo_svg() -> Response {
    asset("image/svg+xml; charset=utf-8", AYAME_LOGO_SVG)
}

pub(super) async fn iris_watercolor_png() -> Response {
    asset_bytes("image/png", IRIS_WATERCOLOR)
}

fn asset(content_type: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}

fn asset_bytes(content_type: &'static str, body: &'static [u8]) -> Response {
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}
