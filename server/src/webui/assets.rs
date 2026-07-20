//! Embedded assets: brand fonts and the Kimün mark, compiled into the binary.

use axum::response::{IntoResponse, Response};

/// Brand fonts served from the binary (single-binary constraint: no CDN, no
/// external requests). Public — the login page needs them too.
pub(super) async fn font_asset(axum::extract::Path(file): axum::extract::Path<String>) -> Response {
    let bytes: &'static [u8] = match file.as_str() {
        "ahm-regular.woff2" => {
            include_bytes!("../../assets/fonts/AtkinsonHyperlegibleMono-Regular.woff2")
        }
        "ahm-bold.woff2" => {
            include_bytes!("../../assets/fonts/AtkinsonHyperlegibleMono-Bold.woff2")
        }
        "inter-regular.woff2" => include_bytes!("../../assets/fonts/Inter-Regular.woff2"),
        "inter-semibold.woff2" => include_bytes!("../../assets/fonts/Inter-SemiBold.woff2"),
        _ => return axum::http::StatusCode::NOT_FOUND.into_response(),
    };
    (
        [
            (axum::http::header::CONTENT_TYPE, "font/woff2"),
            (
                axum::http::header::CACHE_CONTROL,
                "public, max-age=31536000, immutable",
            ),
        ],
        bytes,
    )
        .into_response()
}

/// The Kimün mark (nav brand + favicon), embedded like the fonts.
pub(super) async fn image_asset(
    axum::extract::Path(file): axum::extract::Path<String>,
) -> Response {
    let bytes: &'static [u8] = match file.as_str() {
        "kimun.png" => include_bytes!("../../assets/img/kimun.png"),
        _ => return axum::http::StatusCode::NOT_FOUND.into_response(),
    };
    (
        [
            (axum::http::header::CONTENT_TYPE, "image/png"),
            (
                axum::http::header::CACHE_CONTROL,
                "public, max-age=31536000, immutable",
            ),
        ],
        bytes,
    )
        .into_response()
}
