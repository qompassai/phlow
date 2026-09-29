//! Endpoint, model and optional API key, read from the environment.

use std::fmt;
use std::net::IpAddr;

use phlow_llm::REDACTED;
use reqwest::Url;

use crate::error::System1Error;

/// Environment variable naming the System 1 server base URL.
pub const ENDPOINT_ENV: &str = "PHLOW_SYSTEM1_ENDPOINT";
/// Environment variable naming the model the server should use.
pub const MODEL_ENV: &str = "PHLOW_SYSTEM1_MODEL";
/// Environment variable holding the API key. The key is read only from
/// here, never from files, arguments or model output.
pub const API_KEY_ENV: &str = "PHLOW_SYSTEM1_API_KEY";
/// Default endpoint: a local laya-serve on loopback. No remote default.
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:8000";
/// Default model name sent to the server.
pub const DEFAULT_MODEL: &str = "laya";
/// Maximum bytes of the endpoint, model or key.
pub const CONFIG_VALUE_BYTES_MAX: usize = 512;

/// An API key. Deliberately has no `Display`, `Serialize` or public
/// accessor; `Debug` prints only [`REDACTED`]. The key leaves this crate
/// only as a sensitive `Authorization` header value.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ApiKey(String);

impl ApiKey {
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

/// Where and how to reach the System 1 server.
///
/// `endpoint` and `model` are public so callers can inspect them; they are
/// re-validated by [`crate::HttpBackend::new`], so editing them cannot
/// bypass the endpoint rules. The key is private and only settable from
/// [`API_KEY_ENV`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct System1Config {
    pub endpoint: String,
    pub model: String,
    api_key: Option<ApiKey>,
}

impl System1Config {
    /// Read [`ENDPOINT_ENV`], [`MODEL_ENV`] and [`API_KEY_ENV`] from the
    /// process environment. Unset variables take the loopback defaults;
    /// set-but-invalid variables (including empty or non-UTF-8) are errors.
    pub fn from_env() -> Result<Self, System1Error> {
        Self::from_lookup(|name| match std::env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => Err(System1Error::Config {
                reason: "environment value is not valid UTF-8",
            }),
        })
    }

    /// [`System1Config::from_env`] over an arbitrary variable lookup, so
    /// tests never mutate the process environment.
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Result<Option<String>, System1Error>,
    ) -> Result<Self, System1Error> {
        let endpoint = lookup(ENDPOINT_ENV)?.unwrap_or_else(|| DEFAULT_ENDPOINT.to_owned());
        let model = lookup(MODEL_ENV)?.unwrap_or_else(|| DEFAULT_MODEL.to_owned());
        let api_key = lookup(API_KEY_ENV)?.map(ApiKey);
        let config = System1Config {
            endpoint,
            model,
            api_key,
        };
        config.request_url()?;
        Ok(config)
    }

    pub(crate) fn api_key(&self) -> Option<&ApiKey> {
        self.api_key.as_ref()
    }

    /// Validate everything and return the full `/v1/systemone` URL.
    ///
    /// The endpoint must be absolute `http`/`https` with a host and no
    /// userinfo, query or fragment (userinfo would put a credential in
    /// every URL that reaches an error message). A key is refused over
    /// plain `http` to a non-loopback host, where it would cross the
    /// network in cleartext.
    pub(crate) fn request_url(&self) -> Result<Url, System1Error> {
        check_value(
            &self.model,
            "model must be 1..=512 bytes without control characters",
        )?;
        if let Some(key) = &self.api_key {
            check_value(
                key.expose(),
                "api key must be 1..=512 bytes without control characters",
            )?;
        }
        check_value(
            &self.endpoint,
            "endpoint must be 1..=512 bytes without control characters",
        )?;
        let config = |reason| System1Error::Config { reason };
        let base = Url::parse(&self.endpoint).map_err(|_| config("endpoint is not a valid URL"))?;
        if !matches!(base.scheme(), "http" | "https") {
            return Err(config("endpoint scheme must be http or https"));
        }
        if !base.username().is_empty() || base.password().is_some() {
            return Err(config("endpoint must not carry userinfo"));
        }
        if base.query().is_some() || base.fragment().is_some() {
            return Err(config("endpoint must not carry a query or fragment"));
        }
        if self.api_key.is_some() && base.scheme() == "http" && !is_loopback(&base) {
            return Err(config("api key requires https or a loopback endpoint"));
        }
        let joined = format!("{}/v1/systemone", self.endpoint.trim_end_matches('/'));
        Url::parse(&joined).map_err(|_| config("endpoint is not a valid URL"))
    }
}

fn check_value(value: &str, reason: &'static str) -> Result<(), System1Error> {
    if value.is_empty()
        || value.len() > CONFIG_VALUE_BYTES_MAX
        || value.chars().any(char::is_control)
    {
        return Err(System1Error::Config { reason });
    }
    Ok(())
}

fn is_loopback(url: &Url) -> bool {
    match url.host_str() {
        Some("localhost") => true,
        Some(host) => host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback()),
        None => false,
    }
}
