//! PF-84-S06 slice B: named ChatGPT/OpenAI sign-in accounts.
//!
//! A named account keeps its whole sign-in (`auth.json` contents: ChatGPT
//! tokens or an OpenAI API-key login) in the home's encrypted vault under
//! `provider/<provider>/accounts/<name>/chatgpt_auth`. It adds no keychain
//! item and never reads or writes the `default` account's `auth.json` or
//! `Codex Auth` item. Refreshed tokens are written back to that label only.

use std::fmt;
use std::path::Path;

use codex_vault::ProviderAccountKind;
use codex_vault::ProviderAccountName;
use codex_vault::Vault;
use codex_vault::VaultKeyStorage;
use zeroize::Zeroizing;

use super::storage::AuthDotJson;
use super::storage::AuthStorageBackend;

/// A named sign-in account of an OpenAI-auth provider.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NamedChatgptAccount {
    pub provider_id: String,
    pub name: String,
}

impl fmt::Display for NamedChatgptAccount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.provider_id, self.name)
    }
}

/// One named account's sign-in, in the vault.
pub(super) struct VaultAccountAuthStorage {
    vault: Vault,
    provider_id: String,
    name: ProviderAccountName,
}

impl fmt::Debug for VaultAccountAuthStorage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VaultAccountAuthStorage")
            .field("provider_id", &self.provider_id)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl VaultAccountAuthStorage {
    pub(super) fn new(codex_home: &Path, account: &NamedChatgptAccount) -> std::io::Result<Self> {
        Self::with_vault(Vault::new(codex_home.to_path_buf()), account)
    }

    pub(super) fn with_vault(vault: Vault, account: &NamedChatgptAccount) -> std::io::Result<Self> {
        let name = ProviderAccountName::parse(&account.name)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
        codex_vault::validate_provider_account_provider_id(&account.provider_id)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
        Ok(Self {
            vault,
            provider_id: account.provider_id.clone(),
            name,
        })
    }
}

impl AuthStorageBackend for VaultAccountAuthStorage {
    fn load(&self) -> std::io::Result<Option<AuthDotJson>> {
        // Never initialise a vault (and its key) just to read.
        if self.vault.key_storage() == VaultKeyStorage::NotInitialized {
            return Ok(None);
        }
        let Some(raw) = self
            .vault
            .read_provider_account(
                &self.provider_id,
                &self.name,
                ProviderAccountKind::ChatgptAuth,
            )
            .map_err(std::io::Error::other)?
        else {
            return Ok(None);
        };
        // The parse error never quotes the value.
        serde_json::from_str(&raw).map(Some).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "the stored sign-in of account `{}` is unreadable ({:?} at line {}, column {})",
                    self.name,
                    error.classify(),
                    error.line(),
                    error.column()
                ),
            )
        })
    }

    fn save(&self, auth: &AuthDotJson) -> std::io::Result<()> {
        let raw = Zeroizing::new(serde_json::to_string(auth).map_err(std::io::Error::other)?);
        self.vault
            .write_provider_account(
                &self.provider_id,
                &self.name,
                ProviderAccountKind::ChatgptAuth,
                &raw,
            )
            .map_err(std::io::Error::other)
    }

    fn delete(&self) -> std::io::Result<bool> {
        if self.vault.key_storage() == VaultKeyStorage::NotInitialized {
            return Ok(false);
        }
        self.vault
            .remove_provider_account(&self.provider_id, &self.name)
            .map_err(std::io::Error::other)
    }
}

/// Saves `auth` as the named account's sign-in (creating the account). The
/// default login is never touched.
pub fn save_named_account_auth(
    codex_home: &Path,
    account: &NamedChatgptAccount,
    auth: &AuthDotJson,
) -> std::io::Result<()> {
    VaultAccountAuthStorage::new(codex_home, account)?.save(auth)
}

/// An OpenAI API-key sign-in for a named account.
pub fn login_named_account_with_api_key(
    codex_home: &Path,
    account: &NamedChatgptAccount,
    api_key: &str,
) -> std::io::Result<()> {
    save_named_account_auth(
        codex_home,
        account,
        &AuthDotJson {
            auth_mode: Some(codex_protocol::auth::AuthMode::ApiKey),
            openai_api_key: Some(api_key.to_string()),
            tokens: None,
            last_refresh: None,
            agent_identity: None,
            personal_access_token: None,
            bedrock_api_key: None,
        },
    )
}

/// Revokes the named account's tokens (best effort) and deletes only its
/// sign-in. `Ok(false)` when it had none.
pub async fn logout_named_account_with_revoke(
    codex_home: &Path,
    account: &NamedChatgptAccount,
    auth_route_config: &crate::AuthRouteConfig,
) -> std::io::Result<bool> {
    let storage = VaultAccountAuthStorage::new(codex_home, account)?;
    let auth = storage.load()?;
    if let Err(error) = super::revoke::revoke_auth_tokens(auth.as_ref(), auth_route_config).await {
        tracing::warn!("failed to revoke named account tokens during logout: {error}");
    }
    storage.delete()
}

/// What may be shown about a named sign-in: never a token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedSignInSummary {
    /// `ChatGPT sign-in` or `OpenAI API key`.
    pub kind: &'static str,
    /// The first letter and domain of the email, e.g. `t…@example.com`.
    pub email_hint: Option<String>,
    pub plan: Option<String>,
}

/// The first letter and the domain of `email`.
pub fn email_hint(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) => {
            let first = local.chars().next().map(String::from).unwrap_or_default();
            format!("{first}…@{domain}")
        }
        None => "…".to_string(),
    }
}

pub(super) fn summarize(auth: &AuthDotJson) -> NamedSignInSummary {
    match auth.tokens.as_ref() {
        Some(tokens) => NamedSignInSummary {
            kind: "ChatGPT sign-in",
            email_hint: tokens.id_token.email.as_deref().map(email_hint),
            plan: tokens.id_token.get_chatgpt_plan_type(),
        },
        None => NamedSignInSummary {
            kind: "OpenAI API key",
            email_hint: None,
            plan: None,
        },
    }
}

/// The summary of a named account's sign-in, `None` when it has none.
pub fn named_account_summary(
    codex_home: &Path,
    account: &NamedChatgptAccount,
) -> std::io::Result<Option<NamedSignInSummary>> {
    Ok(VaultAccountAuthStorage::new(codex_home, account)?
        .load()?
        .as_ref()
        .map(summarize))
}

/// Other sign-ins of this home (the `default` login and the provider's other
/// named accounts) that belong to the same ChatGPT user as `account`'s
/// stored sign-in. Signing the same user in twice is allowed (Travis,
/// 2026-10-11: warn, don't refuse), but both accounts then share that user's
/// limits and a sign-out of one can end the other's session.
pub fn same_user_sign_ins(
    codex_home: &Path,
    account: &NamedChatgptAccount,
    default_auth: Option<&AuthDotJson>,
) -> std::io::Result<Vec<String>> {
    let user = |auth: &AuthDotJson| {
        auth.tokens
            .as_ref()
            .and_then(|tokens| tokens.id_token.chatgpt_user_id.clone())
    };
    let Some(target) = VaultAccountAuthStorage::new(codex_home, account)?
        .load()?
        .as_ref()
        .and_then(user)
    else {
        return Ok(Vec::new());
    };
    let mut same = Vec::new();
    if default_auth.and_then(user).as_deref() == Some(target.as_str()) {
        same.push(codex_vault::DEFAULT_PROVIDER_ACCOUNT.to_string());
    }
    let vault = Vault::new(codex_home.to_path_buf());
    if vault.key_storage() == VaultKeyStorage::NotInitialized {
        return Ok(same);
    }
    for other in vault.list_provider_accounts().map_err(std::io::Error::other)? {
        if other.provider_id != account.provider_id
            || other.name.as_str() == account.name
            || !other.kinds.contains(&ProviderAccountKind::ChatgptAuth)
        {
            continue;
        }
        let other_account = NamedChatgptAccount {
            provider_id: other.provider_id.clone(),
            name: other.name.as_str().to_string(),
        };
        let other_user = VaultAccountAuthStorage::new(codex_home, &other_account)?
            .load()?
            .as_ref()
            .and_then(user);
        if other_user.as_deref() == Some(target.as_str()) {
            same.push(other_account.name);
        }
    }
    Ok(same)
}

#[cfg(test)]
#[path = "named_chatgpt_tests.rs"]
mod tests;
