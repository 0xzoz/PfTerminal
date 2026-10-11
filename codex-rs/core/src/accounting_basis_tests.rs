//! The declared billing basis decides a turn's economics (PF-60-S05).
use super::*;
use crate::config::AccountingMode;
use crate::config::PriceAuthority;
use codex_model_provider_info::BillingBasis;
use codex_model_provider_info::ModelProviderInfo;
use codex_protocol::auth::AuthMode;
use codex_protocol::protocol::SessionSource;
use codex_state::SqliteConfig;
use codex_state::ThreadMetadataBuilder;
use codex_state::accounting::Basis;
use codex_state::accounting::BasisSource;
use codex_state::accounting::Rates;
use codex_state::accounting::Snapshot;
use codex_utils_absolute_path::AbsolutePathBuf;
use pretty_assertions::assert_eq;

fn bound(
    id: &str,
    provider: &ModelProviderInfo,
    auth: Option<AuthMode>,
    endpoint: &str,
) -> (PriceAuthority, BasisSource) {
    let mode = AccountingMode::Provider {
        scope: Uuid::new_v4(),
        provider_id: id.into(),
        wire_api: provider.wire_api,
        approved_endpoint: endpoint.into(),
        approved_query: None,
        pricing: PriceAuthority::Unavailable,
        basis_source: Default::default(),
        account: None,
    };
    let AccountingMode::Provider {
        pricing,
        basis_source,
        ..
    } = turn_mode(&mode, id, provider, auth, endpoint)
    else {
        panic!("{id}: provider mode must survive rebinding");
    };
    (pricing, basis_source)
}

fn built_in(id: &str) -> ModelProviderInfo {
    codex_model_provider_info::built_in_model_providers(/*openai_base_url*/ None)[id].clone()
}

fn own_route(id: &str, auth: Option<AuthMode>) -> String {
    built_in(id).to_api_provider(auth).unwrap().base_url
}

/// The declared table, not the kind of login, picks the economics. Each row
/// would fail if inference from `AuthMode::ApiKey` or `provider.auth` came
/// back (review Blocker 1 and Minor 10).
#[test]
fn declared_basis_decides_the_economics() {
    let mut bedrock = built_in("amazon-bedrock");
    bedrock.auth = Some(toml::from_str("command = \"print-token\"").unwrap());
    let custom = ModelProviderInfo {
        name: "custom".into(),
        env_key: Some("CUSTOM_KEY".into()),
        wire_api: codex_model_provider_info::WireApi::Chat,
        ..ModelProviderInfo::default()
    };
    let cases = [
        // Kimi Code membership through an environment key: subscription.
        (
            "kimi-code",
            built_in("kimi-code"),
            Some(AuthMode::ApiKey),
            own_route("kimi-code", None),
            PriceAuthority::PlanRate,
        ),
        (
            "zai-anthropic",
            built_in("zai-anthropic"),
            Some(AuthMode::ApiKey),
            own_route("zai-anthropic", None),
            PriceAuthority::PlanRate,
        ),
        (
            "zai",
            built_in("zai"),
            Some(AuthMode::ApiKey),
            own_route("zai", None),
            PriceAuthority::ApiKeyRates,
        ),
        (
            "claude-plan",
            built_in("claude-plan"),
            None,
            own_route("claude-plan", None),
            PriceAuthority::PlanRate,
        ),
        // A command login on Bedrock is still pay per use; AWS routes carry no
        // attributable catalogue rate.
        (
            "amazon-bedrock",
            bedrock,
            None,
            "https://bedrock-mantle.us-east-1.api.aws/openai/v1".to_string(),
            PriceAuthority::Unavailable,
        ),
        // Realtime: the ChatGPT sign-in at the API host is subscription work.
        (
            "openai",
            built_in("openai"),
            Some(AuthMode::Chatgpt),
            "https://api.openai.com/v1".to_string(),
            PriceAuthority::PlanBasis,
        ),
        // A credential with no declared row.
        (
            "openai",
            built_in("openai"),
            Some(AuthMode::Headers),
            own_route("openai", Some(AuthMode::Headers)),
            PriceAuthority::Undeclared,
        ),
        (
            "ollama",
            built_in("ollama"),
            None,
            own_route("ollama", None),
            PriceAuthority::Local,
        ),
        (
            "openai",
            built_in("openai"),
            Some(AuthMode::Chatgpt),
            own_route("openai", Some(AuthMode::Chatgpt)),
            PriceAuthority::PlanRate,
        ),
        (
            "openai",
            built_in("openai"),
            Some(AuthMode::ApiKey),
            own_route("openai", Some(AuthMode::ApiKey)),
            PriceAuthority::ApiKeyRates,
        ),
        // Option B: a custom provider follows the route it used.
        (
            "my-glm",
            custom.clone(),
            None,
            "https://api.z.ai/api/coding/paas/v4".to_string(),
            PriceAuthority::PlanBasis,
        ),
        (
            "my-llm",
            custom,
            None,
            "https://llm.example.com/v1".to_string(),
            PriceAuthority::Undeclared,
        ),
        // A built-in credential moved off its route is judged by that route.
        (
            "openai",
            built_in("openai"),
            Some(AuthMode::ApiKey),
            "https://relay.invalid/v1".to_string(),
            PriceAuthority::Undeclared,
        ),
    ];
    for (id, provider, auth, endpoint, expected) in cases {
        assert_eq!(
            bound(id, &provider, auth, &endpoint),
            (expected, BasisSource::BuiltIn),
            "{id} at {endpoint} with {auth:?}"
        );
    }
}

/// `model_providers.<id>.billing` overrides the table and is recorded as such.
#[test]
fn configured_basis_overrides_and_is_recorded() {
    let mut zai = built_in("zai");
    zai.billing = Some(BillingBasis::Subscription);
    assert_eq!(
        bound("zai", &zai, Some(AuthMode::ApiKey), &own_route("zai", None)),
        (PriceAuthority::PlanRate, BasisSource::UserConfig)
    );
    let custom = ModelProviderInfo {
        name: "custom".into(),
        billing: Some(BillingBasis::Local),
        ..ModelProviderInfo::default()
    };
    assert_eq!(
        bound("my-box", &custom, None, "http://192.168.1.5:8000/v1"),
        (PriceAuthority::Local, BasisSource::UserConfig)
    );
}

fn mode(pricing: PriceAuthority, basis_source: BasisSource) -> AccountingMode {
    AccountingMode::Provider {
        scope: Uuid::new_v4(),
        provider_id: "my-llm".into(),
        wire_api: codex_model_provider_info::WireApi::Chat,
        approved_endpoint: "https://llm.example.com/v1".into(),
        approved_query: None,
        pricing,
        basis_source,
        account: None,
    }
}

/// Every basis is bound to the attempt at admission, with where it came from:
/// a record with no rates when there is no price - pay per use included, so a
/// user-declared pay-per-use route with no price still says it was declared.
#[tokio::test(flavor = "current_thread")]
async fn admission_binds_the_basis_to_every_attempt() -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let db = StateRuntime::init(
        SqliteConfig::from_sqlite_home(AbsolutePathBuf::try_from(home.path())?),
        "my-llm".into(),
    )
    .await?;
    let owner = ThreadId::new();
    db.upsert_thread(
        &ThreadMetadataBuilder::new(
            owner,
            home.path().join("basis.jsonl"),
            chrono::Utc::now(),
            SessionSource::Cli,
        )
        .build("my-llm"),
    )
    .await?;
    // One UTC day for every read; the cases run within it.
    let day = chrono::Utc::now().timestamp_millis() / 86_400_000;
    let cases = [
        (
            PriceAuthority::Undeclared,
            BasisSource::BuiltIn,
            Some(Basis::Undeclared),
        ),
        (
            PriceAuthority::Local,
            BasisSource::UserConfig,
            Some(Basis::Local),
        ),
        (
            PriceAuthority::PlanBasis,
            BasisSource::UserConfig,
            Some(Basis::PlanEquivalent),
        ),
        (
            PriceAuthority::Unavailable,
            BasisSource::UserConfig,
            Some(Basis::Billed),
        ),
    ];
    for (pricing, source, expected) in cases {
        let sampling = Sampling::start(
            db.clone(),
            owner,
            format!("{pricing:?}"),
            &mode(pricing, source),
        )
        .await?;
        sampling
            .admit("some-model", "https://llm.example.com/v1/chat/completions")
            .await?;
        let quote = codex_state::accounting::AccountingStore::inspect_day(
            &db,
            owner,
            day,
            chrono::Utc::now().timestamp_millis(),
        )
        .await?;
        let codex_state::accounting::InspectionDay::Ready(view) = quote else {
            panic!("{quote:?}");
        };
        let snapshot: Option<Snapshot> = view
            .requests
            .values()
            .flatten()
            .find(|quote| quote.attempt.turn == format!("{pricing:?}"))
            .and_then(|quote| quote.snapshot.clone());
        assert_eq!(
            snapshot.map(|s| (s.basis, s.rates, s.basis_source)),
            expected.map(|basis| (basis, Rates::default(), source)),
            "{pricing:?}"
        );
    }
    // A changed declaration (each case above is a new one for the same
    // provider) applies to new attempts only: every earlier attempt still
    // reads with the basis it was admitted under (AC3).
    let codex_state::accounting::InspectionDay::Ready(view) =
        codex_state::accounting::AccountingStore::inspect_day(
            &db,
            owner,
            day,
            chrono::Utc::now().timestamp_millis(),
        )
        .await?
    else {
        panic!("expected the day");
    };
    let mut bases: Vec<(String, Basis)> = view
        .requests
        .values()
        .flatten()
        .map(|quote| (quote.attempt.turn.clone(), quote.basis()))
        .collect();
    bases.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        bases,
        vec![
            ("Local".to_string(), Basis::Local),
            ("PlanBasis".to_string(), Basis::PlanEquivalent),
            ("Unavailable".to_string(), Basis::Billed),
            ("Undeclared".to_string(), Basis::Undeclared),
        ]
    );
    Ok(())
}

/// Image generation takes OpenAI's published image rates only where API-key
/// rates apply: OpenAI's own provider, pay per use. A plan route, an unpriced
/// route or another provider gets no image rates, and keeps the ordinary
/// usage reading.
#[tokio::test(flavor = "current_thread")]
async fn image_generation_rates_apply_only_to_openai_api_key_rates() -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let db = StateRuntime::init(
        SqliteConfig::from_sqlite_home(AbsolutePathBuf::try_from(home.path())?),
        "openai".into(),
    )
    .await?;
    let owner = ThreadId::new();
    db.upsert_thread(
        &ThreadMetadataBuilder::new(
            owner,
            home.path().join("image.jsonl"),
            chrono::Utc::now(),
            SessionSource::Cli,
        )
        .build("openai"),
    )
    .await?;
    let day = chrono::Utc::now().timestamp_millis() / 86_400_000;
    let endpoint = "https://api.openai.com/v1";
    let image_rates = Rates {
        noncached: Some(serde_json::from_value(serde_json::json!("5"))?),
        read: Some(serde_json::from_value(serde_json::json!("1.25"))?),
        write: None,
        output: Some(serde_json::from_value(serde_json::json!("30"))?),
    };
    let cases = [
        ("openai", PriceAuthority::ApiKeyRates, true, image_rates),
        ("openai", PriceAuthority::PlanRate, false, Rates::default()),
        (
            "openai",
            PriceAuthority::Unavailable,
            false,
            Rates::default(),
        ),
        (
            "openrouter",
            PriceAuthority::ApiKeyRates,
            false,
            Rates::default(),
        ),
    ];
    for (provider, pricing, reads_images, expected) in cases {
        let turn = format!("{provider}-{pricing:?}");
        let mode = AccountingMode::Provider {
            scope: Uuid::new_v4(),
            provider_id: provider.into(),
            wire_api: codex_model_provider_info::WireApi::Responses,
            approved_endpoint: endpoint.into(),
            approved_query: None,
            pricing,
            basis_source: BasisSource::BuiltIn,
            account: None,
        };
        let sampling = Sampling::start_at_path(
            db.clone(),
            owner,
            turn.clone(),
            &mode,
            prices::IMAGE_GENERATIONS_PATH,
        )
        .await?;
        assert_eq!(sampling.image_generation, reads_images, "{turn}");
        sampling
            .admit_with_tier(
                "gpt-image-2",
                "https://api.openai.com/v1/images/generations",
                /*tier*/ None,
            )
            .await?;
        let codex_state::accounting::InspectionDay::Ready(view) =
            codex_state::accounting::AccountingStore::inspect_day(
                &db,
                owner,
                day,
                chrono::Utc::now().timestamp_millis(),
            )
            .await?
        else {
            panic!("expected the day");
        };
        let rates = view
            .requests
            .values()
            .flatten()
            .find(|quote| quote.attempt.turn == turn)
            .and_then(|quote| quote.snapshot.clone())
            .map(|snapshot| snapshot.rates);
        assert_eq!(rates, Some(expected), "{turn}");
    }
    Ok(())
}

/// PF-84-S05: a turn on a named account binds the account's name and per-home
/// fingerprint (never its key), keeps them when the mode is rebound for the
/// same account, and records them on every attempt; the default account
/// records none. The ledger moves to format 4 only once a named attempt lands.
#[tokio::test(flavor = "current_thread")]
async fn admission_records_the_named_account() -> anyhow::Result<()> {
    const CANARY: &str = "canary-pf84-s05-work-key";
    let home = tempfile::tempdir()?;
    let name = codex_vault::ProviderAccountName::parse("work")?;
    let vault = codex_vault::Vault::new(home.path().to_path_buf());
    vault.write_provider_account(
        "my-llm",
        &name,
        codex_vault::ProviderAccountKind::ApiKey,
        CANARY,
    )?;
    let fingerprint = vault
        .provider_account_fingerprint("my-llm", &name)?
        .expect("fingerprint");
    let endpoint = "https://llm.example.com/v1";
    let mut provider = ModelProviderInfo {
        base_url: Some(endpoint.into()),
        wire_api: codex_model_provider_info::WireApi::Chat,
        ..ModelProviderInfo::create_zai_provider()
    };
    let default_mode = turn_mode(
        &mode(PriceAuthority::Unavailable, BasisSource::BuiltIn),
        "my-llm",
        &provider,
        None,
        endpoint,
    );
    provider.account = Some(codex_model_provider_info::NamedProviderAccount {
        provider_id: "my-llm".into(),
        name: "work".into(),
    });
    let mut named_mode = turn_mode(&default_mode, "my-llm", &provider, None, endpoint);
    let account_of = |mode: &AccountingMode| match mode {
        AccountingMode::Provider { account, .. } => account.clone(),
        _ => panic!("{mode:?}"),
    };
    assert_eq!(account_of(&default_mode), None);
    assert_eq!(
        account_of(&named_mode),
        Some(codex_state::accounting::AttemptAccount {
            name: "work".into(),
            fingerprint: None,
        })
    );
    bind_account_fingerprint(&mut named_mode, &provider, home.path().to_path_buf()).await;
    let expected = codex_state::accounting::AttemptAccount {
        name: "work".into(),
        fingerprint: Some(fingerprint),
    };
    assert_eq!(account_of(&named_mode), Some(expected.clone()));
    // Rebinding the same account keeps its fingerprint; another account's does not.
    let rebound = turn_mode(&named_mode, "my-llm", &provider, None, endpoint);
    assert_eq!(account_of(&rebound), Some(expected.clone()));
    let mut other = provider.clone();
    other.account.as_mut().unwrap().name = "other".into();
    assert_eq!(
        account_of(&turn_mode(&named_mode, "my-llm", &other, None, endpoint))
            .and_then(|account| account.fingerprint),
        None
    );
    // Nor does a same-named account of another provider.
    assert_eq!(
        account_of(&turn_mode(
            &named_mode,
            "other-llm",
            &provider,
            None,
            endpoint
        ))
        .and_then(|account| account.fingerprint),
        None
    );

    let db = StateRuntime::init(
        SqliteConfig::from_sqlite_home(AbsolutePathBuf::try_from(home.path())?),
        "my-llm".into(),
    )
    .await?;
    let owner = ThreadId::new();
    db.upsert_thread(
        &ThreadMetadataBuilder::new(
            owner,
            home.path().join("account.jsonl"),
            chrono::Utc::now(),
            SessionSource::Cli,
        )
        .build("my-llm"),
    )
    .await?;
    let formats = || async {
        let pool = db
            .sqlite()
            .open_read_write_pool(&db.sqlite().state_db_path())
            .await?;
        let mut conn = pool.acquire().await?.detach();
        pool.close().await;
        let applied: Vec<i64> =
            sqlx::query_scalar("SELECT version FROM _accounting_migrations ORDER BY version")
                .fetch_all(&mut conn)
                .await?;
        anyhow::Ok(applied)
    };
    let route = format!("{endpoint}/chat/completions");
    let default_attempt = Sampling::start(db.clone(), owner, "default".into(), &default_mode)
        .await?
        .admit("some-model", &route)
        .await?;
    assert_eq!(default_attempt.account, None);
    assert_eq!(formats().await?, vec![1]);
    let named_sampling = Sampling::start(db.clone(), owner, "named".into(), &named_mode).await?;
    // A rebound mode on another account is not this sampling's account.
    assert!(named_sampling.same_account(&rebound));
    assert!(!named_sampling.same_account(&default_mode));
    let named_attempt = named_sampling.admit("some-model", &route).await?;
    assert_eq!(named_attempt.account, Some(expected.clone()));
    // A format-1 ledger goes straight to format 4 with the named attempt.
    assert_eq!(formats().await?, vec![1, 2, 3, 4]);
    let day = chrono::Utc::now().timestamp_millis() / 86_400_000;
    let codex_state::accounting::InspectionDay::Ready(view) =
        codex_state::accounting::AccountingStore::inspect_day(
            &db,
            owner,
            day,
            chrono::Utc::now().timestamp_millis(),
        )
        .await?
    else {
        panic!("expected the day");
    };
    let mut accounts: Vec<(String, Option<codex_state::accounting::AttemptAccount>)> = view
        .requests
        .values()
        .flatten()
        .map(|quote| (quote.attempt.turn.clone(), quote.attempt.account.clone()))
        .collect();
    accounts.sort();
    assert_eq!(
        accounts,
        vec![("default".into(), None), ("named".into(), Some(expected))]
    );
    db.close().await;
    // No file in the home (state DB, its WAL, the encrypted vault) holds the key.
    let mut pending = vec![home.path().to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let bytes = std::fs::read(&path)?;
                assert!(
                    !bytes.windows(CANARY.len()).any(|w| w == CANARY.as_bytes()),
                    "{}",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

/// PF-84-S05: with no vault in the home, the account is recorded by name only
/// and nothing is created (no vault, key or salt).
#[tokio::test(flavor = "current_thread")]
async fn account_fingerprint_without_a_vault_is_name_only_and_creates_nothing() {
    let home = tempfile::tempdir().expect("home");
    let endpoint = "https://llm.example.com/v1";
    let mut provider = ModelProviderInfo {
        base_url: Some(endpoint.into()),
        wire_api: codex_model_provider_info::WireApi::Chat,
        ..ModelProviderInfo::create_zai_provider()
    };
    provider.account = Some(codex_model_provider_info::NamedProviderAccount {
        provider_id: "my-llm".into(),
        name: "work".into(),
    });
    let mut bound = turn_mode(
        &mode(PriceAuthority::Unavailable, BasisSource::BuiltIn),
        "my-llm",
        &provider,
        None,
        endpoint,
    );
    bind_account_fingerprint(&mut bound, &provider, home.path().to_path_buf()).await;
    let AccountingMode::Provider { account, .. } = bound else {
        panic!("{bound:?}");
    };
    assert_eq!(
        account,
        Some(codex_state::accounting::AttemptAccount {
            name: "work".into(),
            fingerprint: None,
        })
    );
    assert_eq!(std::fs::read_dir(home.path()).expect("home").count(), 0);
}
