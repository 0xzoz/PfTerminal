use aws_config::profile::profile_file::ProfileFileKind;
use bytes::Bytes;
use http::HeaderMap;
use http::Method;
use pretty_assertions::assert_eq;

use super::*;
use crate::AwsRequestToSign;
use crate::signing::header_value;

const SERVICE: &str = "bedrock-mantle";

/// A `default` profile, a `work` profile with a region, a `noregion`
/// profile and an SSO-style `sso-dev` profile, all with fake keys.
fn files() -> ProfileFiles {
    ProfileFiles::builder()
        .with_contents(
            ProfileFileKind::Config,
            "[default]\nregion = eu-west-1\n\
             [profile work]\nregion = us-west-2\n\
             [profile noregion]\n",
        )
        .with_contents(
            ProfileFileKind::Credentials,
            "[default]\naws_access_key_id = AKIDDEFAULTPROFILE\naws_secret_access_key = fake-default\n\
             [work]\naws_access_key_id = AKIDWORKPROFILE\naws_secret_access_key = fake-work\n\
             [noregion]\naws_access_key_id = AKIDNOREGION\naws_secret_access_key = fake-noregion\n",
        )
        .build()
}

/// The access key id the signature names.
async fn signing_key_id(context: &AwsAuthContext) -> String {
    let signed = context
        .sign(AwsRequestToSign {
            method: Method::POST,
            url: "https://bedrock-mantle.us-west-2.api.aws/openai/v1/responses".to_string(),
            headers: HeaderMap::new(),
            body: Bytes::from_static(b"{}"),
        })
        .await
        .expect("sign");
    let authorization =
        header_value(&signed.headers, http::header::AUTHORIZATION.as_str()).expect("signed");
    let credential = authorization
        .split("Credential=")
        .nth(1)
        .expect("credential scope");
    credential.split('/').next().unwrap_or_default().to_string()
}

#[tokio::test]
async fn named_profile_uses_only_that_profile() {
    let context = load_named_profile(
        "work",
        /*region_override*/ None,
        SERVICE,
        Some(files()),
    )
    .await
    .expect("work profile");
    assert_eq!(
        (context.region().to_string(), signing_key_id(&context).await),
        ("us-west-2".to_string(), "AKIDWORKPROFILE".to_string())
    );
    let overridden = load_named_profile(
        "work",
        Some(" eu-central-1 ".to_string()),
        SERVICE,
        Some(files()),
    )
    .await
    .expect("region override");
    assert_eq!(overridden.region(), "eu-central-1");
}

#[tokio::test]
async fn missing_profile_fails_without_falling_back_to_default() {
    let error = load_named_profile(
        "gone",
        /*region_override*/ None,
        SERVICE,
        Some(files()),
    )
    .await
    .expect_err("a missing profile never falls back");
    assert!(
        matches!(error, AwsAuthError::Credentials(_)),
        "{}",
        error.detailed_message()
    );
    assert!(
        error.detailed_message().contains("gone"),
        "{}",
        error.detailed_message()
    );
}

#[tokio::test]
async fn profile_without_region_is_refused() {
    let error = load_named_profile(
        "noregion",
        /*region_override*/ None,
        SERVICE,
        Some(files()),
    )
    .await
    .expect_err("no region");
    assert_eq!(
        error.to_string(),
        AwsAuthError::MissingProfileRegion("noregion".to_string()).to_string()
    );
    assert_eq!(
        named_profile_region("noregion", Some("us-east-1".to_string()), Some(files()))
            .await
            .expect("override"),
        "us-east-1"
    );
}
