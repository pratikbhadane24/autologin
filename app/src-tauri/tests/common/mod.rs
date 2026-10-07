//! Fixture "broker" served by wiremock, shared by the real-Chrome tests.
#![allow(dead_code)]

use autologin_lib::broker::registry::ManifestBundle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const LOGIN_PAGE: &str = r##"<!doctype html><html><body>
<form action="/otp" method="get">
  <input id="user" name="user" type="text">
  <input id="pass" name="pass" type="password">
  <button id="go" type="submit">Login</button>
</form>
</body></html>"##;

// Four single-character boxes, like 5Paisa/Nuvama OTP inputs, and a link
// that only some accounts see.
const OTP_PAGE: &str = r##"<!doctype html><html><body>
<a href="#" id="switch" onclick="document.getElementById('boxes').style.display='block'">Switch to PIN</a>
<form action="/finish" method="get">
  <div id="boxes" style="display:none">
    <input class="pin" maxlength="1" name="p1"><input class="pin" maxlength="1" name="p2">
    <input class="pin" maxlength="1" name="p3"><input class="pin" maxlength="1" name="p4">
  </div>
  <button type="submit">Verify</button>
</form>
</body></html>"##;

pub fn bundle(server: &str) -> ManifestBundle {
    let index = format!(
        "schema_version = 1\nmanifest_version = 1\ndefault_tenant = \"t\"\n[tenants.t]\nfixture = \"{server}\"\nbroker_auth_api = \"{server}\"\n"
    );
    let broker = r##"
id = "fixture"
name = "Fixture Broker"
kind = "browser"
[[fields]]
key = "client_id"
label = "ID"
[[fields]]
key = "password"
label = "Password"
secret = true
[[fields]]
key = "mpin"
label = "PIN"
secret = true

[[steps]]
action = "goto"
url = "{tenant.fixture}/login"
[[steps]]
action = "fill"
sel = ["#user"]
value = "{client_id}"
[[steps]]
action = "type"
sel = ["#pass"]
value = "{password}"
[[steps]]
action = "click"
sel = ["#go"]
[[steps]]
action = "optional"
when = ["text=Switch to PIN"]
steps = [{ action = "click", sel = ["text=Switch to PIN"] }]
[[steps]]
action = "fill_split"
sel = ["input.pin"]
value = "{mpin}"
[[steps]]
action = "click"
sel = ["text=Verify"]

[callback]
url_matches = "/add-broker-account/fixture\\?"
endpoint = "/api/sessions/fixture/callback"
body = { code = "{query.code}" }
"##;
    ManifestBundle::from_files([("index.toml", index.as_str()), ("fixture.toml", broker)]).unwrap()
}

pub async fn serve_fixture() -> MockServer {
    let server = MockServer::start().await;
    let html = |body: &str| ResponseTemplate::new(200).set_body_raw(body.as_bytes().to_vec(), "text/html");
    Mock::given(method("GET")).and(path("/login")).respond_with(html(LOGIN_PAGE)).mount(&server).await;
    Mock::given(method("GET")).and(path("/otp")).respond_with(html(OTP_PAGE)).mount(&server).await;
    // The broker "redirects" to the Cirrus callback with a one-time code.
    let callback = format!("{}/add-broker-account/fixture?code=ONE-TIME&state=AB12", server.uri());
    Mock::given(method("GET"))
        .and(path("/finish"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", callback.as_str()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/add-broker-account/fixture"))
        .respond_with(html("<p>Account Saved!</p>"))
        .expect(0) // interception must stop the code from being used here
        .mount(&server)
        .await;
    server
}

