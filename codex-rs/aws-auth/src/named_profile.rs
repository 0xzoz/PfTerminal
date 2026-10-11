//! PF-84-S06: credentials and region from one named AWS profile only.
//!
//! The SDK's default chain reads `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`
//! before any profile, so a named account must not use it. This loads only
//! the shared config and credentials files (honouring `AWS_CONFIG_FILE` and
//! `AWS_SHARED_CREDENTIALS_FILE`), never `AWS_PROFILE`, environment keys or
//! instance metadata, and never falls back to another profile. SSO and
//! `credential_process` profiles work as the SDK resolves them.

// The SDK keeps these names as deprecated aliases of `aws-runtime` types;
// using them avoids a direct dependency on that crate.
#![allow(deprecated)]

use aws_config::meta::region::ProvideRegion;
use aws_config::profile::ProfileFileCredentialsProvider;
use aws_config::profile::ProfileFileRegionProvider;
use aws_config::profile::profile_file::ProfileFiles;
use aws_credential_types::provider::ProvideCredentials;
use aws_credential_types::provider::SharedCredentialsProvider;

use crate::AwsAuthContext;
use crate::AwsAuthError;

/// The region a named profile uses: `region_override` when set, otherwise
/// the profile's own `region`. No environment fallback. `files` replaces
/// the default AWS files (tests only).
pub(crate) async fn named_profile_region(
    profile: &str,
    region_override: Option<String>,
    files: Option<ProfileFiles>,
) -> Result<String, AwsAuthError> {
    if let Some(region) = region_override
        .map(|region| region.trim().to_string())
        .filter(|region| !region.is_empty())
    {
        return Ok(region);
    }
    let mut builder = ProfileFileRegionProvider::builder().profile_name(profile);
    if let Some(files) = files {
        builder = builder.profile_files(files);
    }
    builder
        .build()
        .region()
        .await
        .map(|region| region.to_string())
        .ok_or_else(|| AwsAuthError::MissingProfileRegion(profile.to_string()))
}

/// Loads `profile` and resolves its credentials once, so a missing profile
/// or failing credentials are reported before any request is signed.
pub(crate) async fn load_named_profile(
    profile: &str,
    region_override: Option<String>,
    service: &str,
    files: Option<ProfileFiles>,
) -> Result<AwsAuthContext, AwsAuthError> {
    if service.trim().is_empty() {
        return Err(AwsAuthError::EmptyService);
    }
    let mut builder = ProfileFileCredentialsProvider::builder().profile_name(profile);
    if let Some(files) = files.clone() {
        builder = builder.profile_files(files);
    }
    let provider = builder.build();
    provider.provide_credentials().await?;
    let region = named_profile_region(profile, region_override, files).await?;
    Ok(AwsAuthContext {
        credentials_provider: SharedCredentialsProvider::new(provider),
        region,
        service: service.trim().to_string(),
    })
}

#[cfg(test)]
#[path = "named_profile_tests.rs"]
mod tests;
