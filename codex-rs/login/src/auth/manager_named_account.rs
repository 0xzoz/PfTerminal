//! PF-84-S06 slice B: [`AuthManager`]s bound to one named ChatGPT/OpenAI
//! sign-in account. One manager (and so one refresh lock) is shared per
//! (home, account) in a process; other processes see a refresh through the
//! guarded reload from the vault.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::RwLock;
use std::sync::Weak;

use tokio::sync::Semaphore;
use tokio::sync::watch;

use super::AuthManager;
use super::CachedAuth;
use super::CodexAuth;
use super::OpenAiApiKeyEnv;
use super::ProviderApiKeyCache;
use super::revoke_auth_tokens;
use crate::auth::named_chatgpt::NamedChatgptAccount;
use crate::auth::named_chatgpt::VaultAccountAuthStorage;
use crate::auth::storage::AuthDotJson;
use crate::auth::storage::AuthStorageBackend;

type Registry = HashMap<(PathBuf, NamedChatgptAccount), Weak<AuthManager>>;

static NAMED_ACCOUNT_MANAGERS: LazyLock<Mutex<Registry>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

impl AuthManager {
    /// The manager of one named sign-in account of this home, created on
    /// first use with this manager's settings. It never reads the default
    /// login, `OPENAI_API_KEY`, `CODEX_API_KEY` or externally provided tokens.
    /// Its sign-in is loaded on the first [`AuthManager::auth`]; use
    /// [`AuthManager::for_named_account_loaded`] where the cache is read
    /// before that.
    pub fn for_named_account(&self, account: NamedChatgptAccount) -> Arc<AuthManager> {
        let key = (self.codex_home.clone(), account.clone());
        let mut registry = NAMED_ACCOUNT_MANAGERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        registry.retain(|_, manager| manager.strong_count() > 0);
        if let Some(manager) = registry.get(&key).and_then(Weak::upgrade) {
            return manager;
        }
        let manager = Arc::new(self.new_named_account_manager(account));
        registry.insert(key, Arc::downgrade(&manager));
        manager
    }

    /// [`AuthManager::for_named_account`] with its sign-in already loaded.
    pub async fn for_named_account_loaded(
        &self,
        account: NamedChatgptAccount,
    ) -> Arc<AuthManager> {
        let manager = self.for_named_account(account);
        if manager.auth_cached().is_none() {
            manager.reload().await;
        }
        manager
    }

    /// The named account this manager serves, if any.
    pub fn named_account(&self) -> Option<&NamedChatgptAccount> {
        self.named_account.as_ref()
    }

    fn new_named_account_manager(&self, account: NamedChatgptAccount) -> AuthManager {
        let (auth_change_tx, _auth_change_rx) = watch::channel(0);
        AuthManager {
            codex_home: self.codex_home.clone(),
            inner: RwLock::new(CachedAuth {
                auth: None,
                permanent_refresh_failure: None,
            }),
            auth_change_tx,
            enable_codex_api_key_env: false,
            openai_api_key_env: OpenAiApiKeyEnv::Ignore,
            auth_credentials_store_mode: self.auth_credentials_store_mode,
            keyring_backend_kind: self.keyring_backend_kind,
            forced_chatgpt_workspace_id: RwLock::new(self.forced_chatgpt_workspace_id()),
            chatgpt_base_url: self.chatgpt_base_url.clone(),
            agent_identity_authapi_base_url: self.agent_identity_authapi_base_url.clone(),
            refresh_lock: Semaphore::new(/*permits*/ 1),
            agent_identity_lock: Semaphore::new(/*permits*/ 1),
            agent_identity_bootstrap_cooldown: Default::default(),
            external_auth: RwLock::new(None),
            provider_api_key_cache: RwLock::new(ProviderApiKeyCache::new()),
            auth_route_config: self.auth_route_config.clone(),
            named_account: Some(account),
        }
    }

    fn named_account_storage(&self) -> std::io::Result<Option<Arc<dyn AuthStorageBackend>>> {
        let Some(account) = self.named_account.as_ref() else {
            return Ok(None);
        };
        Ok(Some(Arc::new(VaultAccountAuthStorage::new(
            &self.codex_home,
            account,
        )?)))
    }

    /// The named account's sign-in from its vault store only. `None` when the
    /// account has none (or this is not a named-account manager).
    pub(super) async fn load_named_account_auth(&self) -> Option<CodexAuth> {
        let account = self.named_account.as_ref()?;
        match self.try_load_named_account_auth().await {
            Ok(auth) => auth,
            Err(error) => {
                tracing::warn!(%account, %error, "named account sign-in unavailable");
                None
            }
        }
    }

    async fn try_load_named_account_auth(&self) -> std::io::Result<Option<CodexAuth>> {
        let Some(storage) = self.named_account_storage()? else {
            return Ok(None);
        };
        let Some(auth_dot_json) = storage
            .load()?
            .filter(AuthDotJson::has_primary_auth_material)
        else {
            return Ok(None);
        };
        let mode = auth_dot_json.storage_mode(self.auth_credentials_store_mode);
        CodexAuth::from_auth_dot_json_with_storage(
            &self.codex_home,
            auth_dot_json,
            mode,
            self.chatgpt_base_url.as_deref(),
            self.keyring_backend_kind,
            self.agent_identity_authapi_base_url.as_deref(),
            &self.auth_route_config,
            Some(storage),
        )
        .await
        .map(Some)
    }

    /// Logs out of the named account only: revokes its tokens when asked and
    /// deletes its vault store. `None` when this is not a named manager.
    pub(super) async fn logout_named_account(
        &self,
        revoke: bool,
    ) -> Option<std::io::Result<bool>> {
        let storage = match self.named_account_storage() {
            Ok(storage) => storage?,
            Err(error) => return Some(Err(error)),
        };
        if revoke {
            let auth_dot_json = self
                .auth_cached()
                .and_then(|auth| auth.get_current_auth_json());
            if let Err(error) =
                revoke_auth_tokens(auth_dot_json.as_ref(), &self.auth_route_config).await
            {
                tracing::warn!("failed to revoke named account tokens during logout: {error}");
            }
        }
        let removed = storage.delete();
        self.reload().await;
        Some(removed)
    }
}
