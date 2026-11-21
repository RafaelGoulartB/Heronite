//! The services Epic's launcher talks to, and the client every request of
//! this crate goes through.

use std::time::Duration;

use crate::http::{Body, Http, Request, USER_AGENT};

/// Base URLs of Epic's services (a test points them at a local server).
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub oauth: String,
    pub launcher: String,
    /// Signed download tickets (`launcher-public-service-prod.ak`).
    pub launcher_tickets: String,
    pub catalog: String,
    pub entitlements: String,
    pub ecommerce: String,
    pub datastorage: String,
    pub library: String,
    pub artifact_delivery: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        let https = |host: &str| format!("https://{host}");
        Self {
            oauth: https("account-public-service-prod03.ol.epicgames.com"),
            launcher: https("launcher-public-service-prod06.ol.epicgames.com"),
            launcher_tickets: https("launcher-public-service-prod.ak.epicgames.com"),
            catalog: https("catalog-public-service-prod06.ol.epicgames.com"),
            entitlements: https("entitlement-public-service-prod08.ol.epicgames.com"),
            ecommerce: https("ecommerceintegration-public-service-ecomprod02.ol.epicgames.com"),
            datastorage: https("datastorage-public-service-liveegs.live.use1a.on.epicgames.com"),
            library: https("library-service.live.use1a.on.epicgames.com"),
            artifact_delivery: https("artifact-delivery-service-public-prod.ol.epicgames.com"),
        }
    }
}

impl Endpoints {
    /// Every service at `base` (tests).
    pub fn all_at(base: &str) -> Self {
        let b = base.trim_end_matches('/').to_string();
        Self {
            oauth: b.clone(),
            launcher: b.clone(),
            launcher_tickets: b.clone(),
            catalog: b.clone(),
            entitlements: b.clone(),
            ecommerce: b.clone(),
            datastorage: b.clone(),
            library: b.clone(),
            artifact_delivery: b,
        }
    }
}

/// An Epic client: the HTTP pool, the services and the catalog's language.
#[derive(Clone)]
pub struct Client {
    pub http: Http,
    pub endpoints: Endpoints,
    /// Catalog language (`en`) and country (`US`), legendary's defaults.
    pub locale: String,
    pub country: String,
    /// The launcher's user agent; legendary's API publishes a newer one
    /// ([`crate::legendary_config`]).
    pub user_agent: String,
    /// The OAuth client the sign-in uses.
    pub client_id: String,
    pub(crate) client_secret: String,
}

impl Default for Client {
    fn default() -> Self {
        Self::new(Endpoints::default())
    }
}

impl Client {
    pub fn new(endpoints: Endpoints) -> Self {
        Self {
            http: Http::new(Duration::from_secs(30)),
            endpoints,
            locale: "en".into(),
            country: "US".into(),
            user_agent: USER_AGENT.into(),
            client_id: crate::auth::CLIENT_ID.into(),
            client_secret: crate::auth::CLIENT_SECRET.into(),
        }
    }

    /// A GET with this client's user agent.
    pub fn get<'a>(&'a self, url: &'a str) -> Request<'a> {
        Request { user_agent: &self.user_agent, ..Request::get(url) }
    }

    /// A POST with this client's user agent.
    pub fn post<'a>(&'a self, url: &'a str, body: Body<'a>) -> Request<'a> {
        Request { user_agent: &self.user_agent, ..Request::post(url, body) }
    }
}
