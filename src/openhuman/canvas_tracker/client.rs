use anyhow::{anyhow, bail, Context, Result};
use reqwest::Url;
use serde::de::DeserializeOwned;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanvasEndpoint {
    Courses,
    PlannerItems {
        context_codes: Vec<String>,
    },
    Assignments {
        course_id: String,
    },
    Assignment {
        course_id: String,
        assignment_id: String,
    },
}

#[derive(Debug, Clone)]
pub struct CanvasRequestPolicy {
    base_url: Url,
}

impl CanvasRequestPolicy {
    pub fn new(host: &str) -> Result<Self> {
        let base_url = Url::parse(host).context("[canvas_tracker::client] invalid Canvas host")?;
        if base_url.scheme() != "https" {
            bail!("[canvas_tracker::client] Canvas host must use HTTPS");
        }
        if base_url.host_str().is_none() {
            bail!("[canvas_tracker::client] Canvas host must include a host");
        }
        if base_url.path() != "/" || base_url.query().is_some() || base_url.fragment().is_some() {
            bail!(
                "[canvas_tracker::client] Canvas host must not include a path, query, or fragment"
            );
        }

        Ok(Self { base_url })
    }

    pub fn url_for(&self, endpoint: CanvasEndpoint) -> Result<Url> {
        let mut url = self.base_url.clone();
        url.set_query(None);
        url.set_fragment(None);

        match endpoint {
            CanvasEndpoint::Courses => {
                url.set_path("/api/v1/courses");
            }
            CanvasEndpoint::PlannerItems { context_codes } => {
                url.set_path("/api/v1/planner/items");
                {
                    let mut pairs = url.query_pairs_mut();
                    for context_code in context_codes {
                        validate_path_id("context_code", &context_code)?;
                        pairs.append_pair("context_codes[]", &context_code);
                    }
                }
            }
            CanvasEndpoint::Assignments { course_id } => {
                validate_path_id("course_id", &course_id)?;
                set_path_segments(
                    &mut url,
                    &["api", "v1", "courses", &course_id, "assignments"],
                )?;
                append_submission_include(&mut url);
            }
            CanvasEndpoint::Assignment {
                course_id,
                assignment_id,
            } => {
                validate_path_id("course_id", &course_id)?;
                validate_path_id("assignment_id", &assignment_id)?;
                set_path_segments(
                    &mut url,
                    &[
                        "api",
                        "v1",
                        "courses",
                        &course_id,
                        "assignments",
                        &assignment_id,
                    ],
                )?;
                append_submission_include(&mut url);
            }
        }

        self.validate_url(url.as_str())
    }

    pub fn validate_url(&self, value: &str) -> Result<Url> {
        let url = Url::parse(value).context("[canvas_tracker::client] invalid Canvas URL")?;
        if url.scheme() != "https" {
            bail!("[canvas_tracker::client] Canvas URL must use HTTPS");
        }
        if url.host_str() != self.base_url.host_str()
            || url.port_or_known_default() != self.base_url.port_or_known_default()
        {
            bail!("[canvas_tracker::client] URL must stay on configured Canvas host");
        }
        if !is_allowed_canvas_path(url.path()) {
            bail!("[canvas_tracker::client] Canvas endpoint is not allowed");
        }

        Ok(url)
    }
}

pub struct CanvasClient {
    http: reqwest::Client,
    policy: CanvasRequestPolicy,
    token: String,
}

impl CanvasClient {
    pub fn new(host: &str, token: String) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::new(),
            policy: CanvasRequestPolicy::new(host)?,
            token,
        })
    }

    pub async fn get_json<T: DeserializeOwned>(&self, endpoint: CanvasEndpoint) -> Result<T> {
        let url = self.policy.url_for(endpoint)?;
        let response = self
            .http
            .get(url.clone())
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|err| anyhow!(redact_token(&err.to_string(), &self.token)))
            .with_context(|| format!("[canvas_tracker::client] GET failed for {url}"))?;

        let response = response
            .error_for_status()
            .map_err(|err| anyhow!(redact_token(&err.to_string(), &self.token)))
            .with_context(|| format!("[canvas_tracker::client] GET returned an error for {url}"))?;

        response
            .json::<T>()
            .await
            .map_err(|err| anyhow!(redact_token(&err.to_string(), &self.token)))
            .with_context(|| format!("[canvas_tracker::client] failed to parse JSON from {url}"))
    }
}

pub fn redact_token(message: &str, token: &str) -> String {
    if token.is_empty() {
        return message.to_string();
    }
    message.replace(token, "[REDACTED]")
}

fn set_path_segments(url: &mut Url, segments: &[&str]) -> Result<()> {
    let mut path_segments = url
        .path_segments_mut()
        .map_err(|()| anyhow!("[canvas_tracker::client] Canvas URL cannot be a base"))?;
    path_segments.clear();
    path_segments.extend(segments.iter().copied());
    Ok(())
}

fn append_submission_include(url: &mut Url) {
    url.query_pairs_mut().append_pair("include[]", "submission");
}

fn validate_path_id(name: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains('?')
        || value.contains('#')
        || value.contains("..")
    {
        bail!("[canvas_tracker::client] invalid {name}");
    }
    Ok(())
}

fn is_allowed_canvas_path(path: &str) -> bool {
    let segments: Vec<_> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    match segments.as_slice() {
        ["api", "v1", "courses"] => true,
        ["api", "v1", "planner", "items"] => true,
        ["api", "v1", "courses", course_id, "assignments"] => {
            validate_path_id("course_id", course_id).is_ok()
        }
        ["api", "v1", "courses", course_id, "assignments", assignment_id] => {
            validate_path_id("course_id", course_id).is_ok()
                && validate_path_id("assignment_id", assignment_id).is_ok()
        }
        _ => false,
    }
}
