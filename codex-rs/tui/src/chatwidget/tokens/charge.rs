//! How `/cost` prices an OpenRouter request (#368; Travis, 2026-10-14).
//!
//! OpenRouter's price depends on which upstream endpoint served a request, so
//! no single catalogue rate is right for every request. OpenRouter states what
//! it charged with each response (`usage.cost`, recorded as `billed_usd`), so
//! a request is priced from that charge. Without one, the catalogue rate bound
//! at dispatch stands, shown as an estimate; with neither, there is no price.
//!
//! The ledger is unchanged: its quote keeps the catalogue estimate and the
//! charge stays in the attempt's usage. This view puts the charge in as the
//! attempt's figure before anything is summed, so every total, page and link
//! reads the same amount, and each page says which basis it used.

use codex_state::accounting::BucketQuote;
use codex_state::accounting::DayTotals;
use codex_state::accounting::Decimal;
use codex_state::accounting::Inspection;
use codex_state::accounting::InspectionDay;
use codex_state::accounting::ObservationQuote;

use super::lacks_price;
use super::money;
use super::per_use;
use super::provider_name;

/// Routes priced from the charge the provider reports with each response.
/// Only OpenRouter: the other routes that state a charge (the Vercel gateway,
/// the Corbanu API) keep it as a billed figure beside the estimate.
fn prices_from_charge(provider: &str) -> bool {
    provider == codex_model_provider_info::OPENROUTER_PROVIDER_ID
}

/// The provider whose reported charges price its requests, by display name.
pub(super) fn charging_provider() -> String {
    provider_name(codex_model_provider_info::OPENROUTER_PROVIDER_ID)
}

/// The charge this attempt is priced from: pay-per-use work on a route priced
/// from its reported charges, not refused, whose response stated one.
pub(super) fn reported_charge(quote: &ObservationQuote) -> Option<Decimal> {
    (prices_from_charge(&quote.attempt.provider)
        && per_use(quote)
        && quote.usage.refused_status.is_none())
    .then_some(quote.usage.billed_usd)
    .flatten()
}

/// What the published prices put the attempt at, the figure a reported charge
/// replaces, or why there is no such figure.
pub(super) fn published_estimate(quote: &ObservationQuote) -> String {
    if quote
        .buckets
        .iter()
        .any(|bucket| matches!(bucket, BucketQuote::MissingRate))
    {
        return "none — no published price covers every component".to_string();
    }
    if quote
        .buckets
        .iter()
        .any(|bucket| matches!(bucket, BucketQuote::MissingUsage))
    {
        return "not available — the response did not report every token count it needs"
            .to_string();
    }
    quote
        .buckets
        .iter()
        .try_fold(Decimal::default(), |sum, bucket| match bucket {
            BucketQuote::Priced(amount) => sum.add(*amount).ok(),
            BucketQuote::MissingUsage | BucketQuote::MissingRate => None,
        })
        .map_or_else(
            || "unavailable — exact arithmetic overflow".to_string(),
            money,
        )
}

/// Price the attempt from its reported charge. Returns whether it changed;
/// applying it again changes nothing.
fn reprice(quote: &mut ObservationQuote) -> bool {
    let Some(charge) = reported_charge(quote) else {
        return false;
    };
    if quote.known_subtotal == charge && quote.all_buckets_priced == Some(charge) {
        return false;
    }
    quote.known_subtotal = charge;
    quote.all_buckets_priced = Some(charge);
    quote.subtotal_display = charge.display();
    true
}

/// Price every attempt in `view` from its reported charge where it has one.
/// Returns whether any attempt in the view's own totals changed.
fn reprice_view(view: &mut Inspection) -> bool {
    let mut changed = false;
    for quote in view
        .requests
        .values_mut()
        .chain(view.unknown_parent_requests.values_mut())
        .flatten()
    {
        changed |= reprice(quote);
    }
    if let Some(others) = &mut view.other_conversations {
        // Other conversations have no stored totals here; their figures are
        // summed from these quotes when shown.
        for quote in others.requests.values_mut().flatten() {
            reprice(quote);
        }
    }
    changed
}

/// `day` with every attempt priced from its reported charge where it has one.
/// A single day's totals are summed again where any attempt changed, and fail
/// only when the exact sums overflow. A range's days keep their stored totals:
/// the range view sums every total it shows from the attempts, bucket by
/// bucket, so an overflow there stays confined to its bucket.
pub(super) fn priced_from_charges(day: InspectionDay) -> anyhow::Result<InspectionDay> {
    Ok(match day {
        InspectionDay::Ready(mut view) => {
            if reprice_view(&mut view) {
                let owner = view.owner;
                let all = || view.requests.values().flatten();
                view.totals = DayTotals::from_quotes(all())?;
                view.own_totals =
                    DayTotals::from_quotes(all().filter(|q| q.attempt.thread_id == owner))?;
                view.descendant_totals =
                    DayTotals::from_quotes(all().filter(|q| q.attempt.thread_id != owner))?;
                view.unknown_parent_totals =
                    DayTotals::from_quotes(view.unknown_parent_requests.values().flatten())?;
            }
            InspectionDay::Ready(view)
        }
        InspectionDay::Range {
            requested,
            aggregate_day_floor,
            read_at_ms,
            mut buckets,
        } => {
            for day in buckets.iter_mut().flat_map(|bucket| &mut bucket.days) {
                if let InspectionDay::Ready(view) = day {
                    reprice_view(view);
                }
            }
            InspectionDay::Range {
                requested,
                aggregate_day_floor,
                read_at_ms,
                buckets,
            }
        }
        other => other,
    })
}

/// How one attempt without a reported charge is priced.
enum Fallback {
    /// Every component priced from published prices.
    Estimate,
    /// Only some components priced: a lower bound.
    Partial,
    /// No figure at all.
    None,
}

fn fallback(quote: &ObservationQuote) -> Fallback {
    if quote.all_buckets_priced.is_some() {
        Fallback::Estimate
    } else if quote.known_subtotal != Decimal::default() {
        Fallback::Partial
    } else {
        Fallback::None
    }
}

/// "Price basis: …" for pages holding work on a route priced from reported
/// charges: whether each figure is the provider's charge, an estimate from
/// published prices, or no price. `None` for any other page, whose figures
/// already say they are estimates.
pub(super) fn price_basis(quotes: &[&ObservationQuote]) -> Option<String> {
    let billable: Vec<&ObservationQuote> = quotes
        .iter()
        .copied()
        .filter(|quote| per_use(quote) && quote.usage.refused_status.is_none())
        .collect();
    if !billable
        .iter()
        .any(|quote| prices_from_charge(&quote.attempt.provider))
    {
        return None;
    }
    let provider = charging_provider();
    // A single attempt here is on the charge-priced route.
    if let [quote] = billable.as_slice() {
        if reported_charge(quote).is_some() {
            return Some(format!(
                "Price basis: the charge {provider} reported with the response (usage.cost)"
            ));
        }
        return Some(match fallback(quote) {
            Fallback::Estimate => format!(
                "Price basis: estimate from published prices — {provider} reported no charge for this response"
            ),
            Fallback::Partial => format!(
                "Price basis: partial estimate from published prices, a lower bound — {provider} reported no charge for this response"
            ),
            Fallback::None => format!(
                "Price basis: none — {provider} reported no charge, and {}",
                if lacks_price(quote) {
                    "no published price covers this response"
                } else {
                    "the response did not report every token count its published price needs"
                }
            ),
        });
    }
    let (mut charged, mut estimated, mut partial, mut silent, mut none) = (0, 0, 0, 0, 0);
    for quote in &billable {
        if reported_charge(quote).is_some() {
            charged += 1;
            continue;
        }
        match fallback(quote) {
            Fallback::Estimate => estimated += 1,
            Fallback::Partial => partial += 1,
            Fallback::None => none += 1,
        }
        if prices_from_charge(&quote.attempt.provider) {
            silent += 1;
        }
    }
    let attempts = |n: usize| format!("{n} {}", if n == 1 { "attempt" } else { "attempts" });
    let mut parts = Vec::new();
    if charged > 0 {
        parts.push(format!(
            "{} priced from the charge {provider} reported",
            attempts(charged)
        ));
    }
    if estimated > 0 {
        parts.push(format!(
            "{} estimated from published prices",
            attempts(estimated)
        ));
    }
    if partial > 0 {
        parts.push(format!(
            "{} estimated in part from published prices (a lower bound)",
            attempts(partial)
        ));
    }
    if none > 0 {
        parts.push(format!("{} with no price", attempts(none)));
    }
    let mut basis = format!("Price basis: {}", parts.join("; "));
    if silent > 0 {
        basis.push_str(&format!(
            " — {provider} reported no charge for {}",
            attempts(silent)
        ));
    }
    Some(basis)
}

#[cfg(test)]
#[path = "charge_tests.rs"]
mod tests;
