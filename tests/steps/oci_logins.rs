//! Steps of the OCI login scenarios (S7b, slice 3): what `docker login` left in
//! a Docker config, the credential helpers it names, a registry that wants a
//! login for a repository prefix, and what the registry's token realm was
//! asked. The registry, the project and what `lvpm` printed are the publish
//! and OCI install steps'.
//!
//! The Docker config is written again after every step that changes it, and the
//! children are pointed at it, so a bare `I run "lvpm lock"` runs against the
//! config the scenario describes.
//!
//! A secret is a long unique string in the feature, so that "does not contain"
//! cannot match by chance.

use crate::CliWorld;
use cucumber::{given, then};
use lvpm_testkit::helper::{self, Behavior, NOT_FOUND};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::LazyLock;

/// The name of the helper the steps that name none install.
const HELPER: &str = "lvpmtest";
/// The user name with which a helper says its secret is an identity token.
const IDENTITY_USER: &str = "<token>";
/// What the helper of "counts its runs" answers.
const DEFAULT_LOGIN: (&str, &str) = ("u1", "s1-7f3a9c-secret");

/// What a login scenario has put in the Docker config.
#[derive(Debug, Default)]
pub struct LoginState {
    /// `credHelpers`: the host, which may be `<registry>`, and the helper's name.
    cred_helpers: Vec<(String, String)>,
    /// `credsStore`.
    creds_store: Option<String>,
    /// `auths`: the host and `user:secret`.
    auths: Vec<(String, String)>,
    /// The secret each helper was told to give for a user name.
    secrets: BTreeMap<String, String>,
}

impl CliWorld {
    /// `config.json` from what the scenario has said, and the children pointed
    /// at it and at the helpers. Nothing said, no file.
    fn write_docker_config(&self) {
        let state = &self.logins;
        let mut config = Map::new();
        if !state.cred_helpers.is_empty() {
            let named: Map<String, Value> = (state.cred_helpers.iter())
                .map(|(host, name)| (self.with_placeholders(host), json!(name)))
                .collect();
            config.insert("credHelpers".into(), named.into());
        }
        if let Some(store) = &state.creds_store {
            config.insert("credsStore".into(), json!(store));
        }
        if !state.auths.is_empty() {
            let held: Map<String, Value> = (state.auths.iter())
                .map(|(host, login)| {
                    let (user, secret) = login.split_once(':').expect("user:secret");
                    let auth = helper::auths_entry(user, secret);
                    (self.with_placeholders(host), json!({ "auth": auth }))
                })
                .collect();
            config.insert("auths".into(), held.into());
        }
        self.sandbox.use_docker_config();
        let file = self.sandbox.docker_config_dir().join("config.json");
        match config.is_empty() {
            true => drop(std::fs::remove_file(&file)),
            false => std::fs::write(&file, Value::Object(config).to_string())
                .expect("write the Docker config"),
        }
    }

    /// `docker-credential-<name>` in the scenario's helper directory, doing `behavior`.
    fn install_helper(&mut self, name: &str, behavior: Behavior) {
        if let Behavior::Answers { user, secret } = &behavior {
            self.logins.secrets.insert(user.clone(), secret.clone());
        }
        helper::install(&self.sandbox.helper_dir(), name, &behavior).expect("install the helper");
        self.write_docker_config();
    }
}

// ---- the registry ----

#[given(regex = r#"^the registry lets anyone read "([^"]+)"$"#)]
async fn registry_lets_anyone_read(_world: &mut CliWorld, _prefix: String) {
    // What the fake does until it is told otherwise.
}

#[given(regex = r#"^the registry requires a login for "([^"]+)"$"#)]
async fn registry_requires_a_login(world: &mut CliWorld, prefix: String) {
    world.registry().require_login(&prefix);
}

#[given(regex = r#"^the registry requires a login for "([^"]+)" and refuses "([^"]+)"$"#)]
async fn registry_requires_a_login_and_refuses(world: &mut CliWorld, prefix: String, user: String) {
    world.registry().require_login(&prefix);
    world.registry().refuse_user(&user);
}

#[given(regex = r#"^the registry requires a login to write "([^"]+)"$"#)]
async fn registry_requires_a_login_to_write(world: &mut CliWorld, prefix: String) {
    world.registry().require_login_to_write(&prefix);
}

#[given(regex = r#"^the registry refuses writes to "([^"]+)"$"#)]
async fn registry_refuses_writes(world: &mut CliWorld, prefix: String) {
    world.registry().refuse_writes_under(&prefix, 403);
}

// ---- the Docker config ----

/// The sandbox's children get an empty Docker config directory: no file in it,
/// and no helper on their `PATH` but the ones a scenario installs.
#[given("no Docker config")]
async fn no_docker_config(world: &mut CliWorld) {
    world.logins = LoginState::default();
    world.write_docker_config();
}

static NAMES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^credHelpers names "([^"]+)" for "([^"]+)"$"#).expect("the pattern")
});
static STORE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^credsStore is "([^"]+)"$"#).expect("the pattern"));
static HOLDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^auths holds "([^":]+):([^"]+)"(?: for "([^"]+)")?$"#).expect("the pattern")
});
static CLAUSES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r", (?:and )?whose ").expect("the pattern"));

/// `credHelpers names "first" for "other.example", whose credsStore is "second", and whose auths holds "u3:s3"`
#[given(regex = r#"^a Docker config whose ((?:credHelpers names|credsStore is|auths holds) .+)$"#)]
async fn docker_config_whose(world: &mut CliWorld, clauses: String) {
    for clause in CLAUSES.split(&clauses) {
        if let Some(found) = NAMES.captures(clause) {
            let named = (found[2].to_string(), found[1].to_string());
            world.logins.cred_helpers.push(named);
        } else if let Some(found) = STORE.captures(clause) {
            world.logins.creds_store = Some(found[1].to_string());
        } else if let Some(found) = HOLDS.captures(clause) {
            let host = found.get(3).map_or("<registry>", |host| host.as_str());
            let login = format!("{}:{}", &found[1], &found[2]);
            world
                .logins
                .secrets
                .insert(found[1].to_string(), found[2].to_string());
            world.logins.auths.push((host.to_string(), login));
        } else {
            panic!("{clause:?} is no clause of a Docker config");
        }
    }
    world.write_docker_config();
}

/// A Docker config with one entry, and the login that entry gives.
#[given(
    regex = r#"^a Docker config whose ((?:credHelpers entry|credsStore|auths entry)(?: "[^"]+")?) gives user "([^"]+)" and secret "([^"]+)" for "([^"]+)"$"#
)]
async fn docker_config_gives(
    world: &mut CliWorld,
    entry: String,
    user: String,
    secret: String,
    host: String,
) {
    let name = || {
        let quoted = entry.split('"').nth(1);
        quoted.expect("a helper name in quotes").to_string()
    };
    match entry.split_whitespace().next() {
        Some("credHelpers") => {
            world.logins.cred_helpers.push((host, name()));
            world.install_helper(&name(), Behavior::Answers { user, secret });
        }
        Some("credsStore") => {
            world.logins.creds_store = Some(name());
            world.install_helper(&name(), Behavior::Answers { user, secret });
        }
        _ => {
            world.logins.secrets.insert(user.clone(), secret.clone());
            world.logins.auths.push((host, format!("{user}:{secret}")));
            world.write_docker_config();
        }
    }
}

// ---- the helpers ----

/// What a helper does, as a feature says it.
fn behavior_of(text: &str) -> Behavior {
    static RETURNS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"^returns user "([^"]+)" and secret "([^"]+)"$"#).expect("the pattern")
    });
    static IDENTITY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"^returns the identity-token user name and secret "([^"]+)"$"#)
            .expect("the pattern")
    });
    static EXITS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"^exits (\d+) printing "([^"]+)" on stderr$"#).expect("the pattern")
    });
    static ANSWERS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"^answers "([^"]+)"$"#).expect("the pattern"));
    if let Some(found) = RETURNS.captures(text) {
        let (user, secret) = (found[1].to_string(), found[2].to_string());
        return Behavior::Answers { user, secret };
    }
    if let Some(found) = IDENTITY.captures(text) {
        let user = IDENTITY_USER.to_string();
        return Behavior::Answers {
            user,
            secret: found[1].to_string(),
        };
    }
    if let Some(found) = EXITS.captures(text) {
        let code = found[1].parse().expect("an exit code");
        let stderr = found[2].to_string();
        return Behavior::Fails { code, stderr };
    }
    if let Some(found) = ANSWERS.captures(text) {
        assert_eq!(
            &found[1], NOT_FOUND,
            "the only answer of a helper with no login"
        );
        return Behavior::HoldsNothing;
    }
    match text {
        "never answers" => Behavior::Hangs,
        "counts its runs" => Behavior::Answers {
            user: DEFAULT_LOGIN.0.to_string(),
            secret: DEFAULT_LOGIN.1.to_string(),
        },
        other => panic!("{other:?} is no behavior of a credential helper"),
    }
}

#[given(regex = r#"^a docker-credential-(\w+) that (.+)$"#)]
async fn helper_named_that(world: &mut CliWorld, name: String, does: String) {
    world.install_helper(&name, behavior_of(&does));
}

/// The helper of a scenario that names none, which is the Docker config's `credsStore`.
#[given(regex = r#"^a credential helper that (.+)$"#)]
async fn a_credential_helper_that(world: &mut CliWorld, does: String) {
    world
        .logins
        .creds_store
        .get_or_insert_with(|| HELPER.to_string());
    world.install_helper(HELPER, behavior_of(&does));
}

#[given(regex = r#"^no docker-credential-(\w+) on the PATH$"#)]
async fn no_helper_on_the_path(world: &mut CliWorld, name: String) {
    world.write_docker_config();
    let wanted = format!("docker-credential-{name}");
    let installed = std::fs::read_dir(world.sandbox.helper_dir()).expect("the helper directory");
    let found = installed
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().starts_with(&wanted));
    assert!(!found, "{wanted} is installed");
}

#[given(regex = r#"^LVPM_OCI_HELPER_TIMEOUT_SECS is (\d+)$"#)]
async fn helper_timeout(world: &mut CliWorld, secs: String) {
    world
        .sandbox
        .set_child_env("LVPM_OCI_HELPER_TIMEOUT_SECS", &secs);
}

// ---- what the registry was asked ----

#[then("the registry received a token request with no Authorization header")]
async fn token_request_without_login(world: &mut CliWorld) {
    let asked = world.registry().token_requests();
    assert!(!asked.is_empty(), "no token request");
    assert!(
        asked.iter().all(|ask| ask.authorization.is_none()),
        "{asked:?}"
    );
}

/// The secret is the one the helper or the config gave for that user name.
#[then(
    regex = r#"^the registry's token endpoint received the login "([^"]+)"( with push scope)?$"#
)]
async fn token_endpoint_got_login(world: &mut CliWorld, user: String, push: String) {
    let asked = world.registry().token_requests();
    let secret = world.logins.secrets.get(&user);
    let logged_in = |ask: &lvpm_testkit::oci::TokenRequest| {
        let pushing = push.is_empty() || ask.scope().ends_with(":pull,push");
        let login = ask.basic_login();
        let right = login.as_ref().is_some_and(|(have, sent)| {
            *have == user && secret.is_none_or(|secret| secret == sent)
        });
        right && pushing
    };
    assert!(asked.iter().any(logged_in), "no such login in {asked:?}");
}

#[then(regex = r#"^the token endpoint received a refresh_token grant with "([^"]+)"$"#)]
async fn token_endpoint_got_refresh_grant(world: &mut CliWorld, token: String) {
    let asked = world.registry().token_requests();
    let granted = |ask: &lvpm_testkit::oci::TokenRequest| ask.refresh_token() == Some(&token);
    assert!(
        asked.iter().any(granted),
        "no refresh_token grant in {asked:?}"
    );
}

#[then("the helper ran once")]
async fn helper_ran_once(world: &mut CliWorld) {
    let ran = helper::runs(&world.sandbox.helper_dir(), HELPER);
    assert_eq!(
        ran,
        [world.registry().host()],
        "the helper ran for these hosts"
    );
}

#[then(regex = r#"^the registry holds no tag "([^"]+)" in "([^"]+)"$"#)]
async fn registry_holds_no_tag(world: &mut CliWorld, tag: String, repository: String) {
    assert_eq!(world.registry().tag_digest(&repository, &tag), None);
}
