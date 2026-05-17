use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::{Path as AxumPath, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use tokio_util::io::ReaderStream;

use crate::openhuman::http_host::auth::ensure_authorized;
use crate::openhuman::http_host::path_utils::{
    child_href_for, content_type_for_path, escape_html, parent_href_for, resolve_request_path,
};
use crate::openhuman::http_host::types::HostedDirAuth;
use crate::openhuman::http_host::LOG_PREFIX;

#[derive(Clone)]
pub(crate) struct HostedDirState {
    pub(crate) root_dir: PathBuf,
    pub(crate) auth: HostedDirAuth,
}

pub(crate) fn build_router(state: HostedDirState) -> Router {
    Router::new()
        .route("/", get(serve_root).head(serve_root))
        .route("/{*path}", get(serve_path).head(serve_path))
        .with_state(state)
}

async fn serve_root(State(state): State<HostedDirState>, headers: HeaderMap) -> Response {
    serve_relative_path(state, headers, "").await
}

async fn serve_path(
    AxumPath(path): AxumPath<String>,
    State(state): State<HostedDirState>,
    headers: HeaderMap,
) -> Response {
    serve_relative_path(state, headers, &path).await
}

async fn serve_relative_path(state: HostedDirState, headers: HeaderMap, path: &str) -> Response {
    if let Err(response) = ensure_authorized(&headers, &state.auth) {
        return response;
    }

    let resolved = match resolve_request_path(&state.root_dir, path) {
        Ok(path) => path,
        Err(error) => {
            log::warn!("{LOG_PREFIX} rejected path='{}': {}", path, error);
            return (StatusCode::BAD_REQUEST, error).into_response();
        }
    };

    match tokio::fs::metadata(&resolved).await {
        Ok(metadata) if metadata.is_dir() => {
            serve_directory(&state.root_dir, &resolved, path).await
        }
        Ok(metadata) if metadata.is_file() => serve_file(&resolved).await,
        Ok(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            (StatusCode::NOT_FOUND, "not found").into_response()
        }
        Err(error) => {
            log::warn!(
                "{LOG_PREFIX} metadata failed path={} err={}",
                resolved.display(),
                error
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to read hosted directory entry",
            )
                .into_response()
        }
    }
}

async fn serve_file(path: &Path) -> Response {
    match tokio::fs::File::open(path).await {
        Ok(file) => {
            let stream = ReaderStream::new(file);
            let mut response = Response::new(Body::from_stream(stream));
            *response.status_mut() = StatusCode::OK;
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static(content_type_for_path(path)),
            );
            response
        }
        Err(error) => {
            log::warn!(
                "{LOG_PREFIX} read file failed path={} err={}",
                path.display(),
                error
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to read hosted file",
            )
                .into_response()
        }
    }
}

async fn serve_directory(root_dir: &Path, dir: &Path, requested_path: &str) -> Response {
    let index_path = dir.join("index.html");
    if tokio::fs::metadata(&index_path)
        .await
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
    {
        return serve_file(&index_path).await;
    }

    match tokio::fs::read_dir(dir).await {
        Ok(mut entries) => {
            let mut rows = Vec::new();
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name().to_string_lossy().to_string();
                let file_type = match entry.file_type().await {
                    Ok(file_type) => file_type,
                    Err(_) => continue,
                };
                let suffix = if file_type.is_dir() { "/" } else { "" };
                rows.push((name, suffix.to_string()));
            }
            rows.sort_by(|a, b| a.0.cmp(&b.0));

            let title = if requested_path.is_empty() {
                "/".to_string()
            } else {
                format!("/{}", requested_path.trim_matches('/'))
            };
            let mut html = String::from(
                "<!doctype html><html><head><meta charset=\"utf-8\"><title>OpenHuman Directory Listing</title></head><body>",
            );
            html.push_str(&format!(
                "<h1>Directory listing for {}</h1><ul>",
                escape_html(&title)
            ));
            if dir != root_dir {
                let parent_href = parent_href_for(requested_path);
                html.push_str(&format!("<li><a href=\"{}\">..</a></li>", parent_href));
            }
            for (name, suffix) in rows {
                let href = child_href_for(requested_path, &name, suffix.as_str());
                html.push_str(&format!(
                    "<li><a href=\"{}\">{}{}</a></li>",
                    href,
                    escape_html(&name),
                    suffix
                ));
            }
            html.push_str("</ul></body></html>");
            Html(html).into_response()
        }
        Err(error) => {
            log::warn!(
                "{LOG_PREFIX} read_dir failed path={} err={}",
                dir.display(),
                error
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to enumerate hosted directory",
            )
                .into_response()
        }
    }
}
