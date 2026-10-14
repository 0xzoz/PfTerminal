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
use codex_state::accounting::InspectionDay;
use codex_state::accounting::ObservationQuote;

use super::lacks_price;
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

/// What the published prices put the attempt at when they price every
/// bucket: the figure a reported charge replaces.
pub(super) fn published_estimate(quote: &ObservationQuote) -> Option<Decimal> {
    quote
        .buckets
        .iter()
        .try_fold(Decimal::default(), |sum, bucket| match bucket {
            BucketQuote::Priced(amount) => sum.add(*amount).ok(),
            BucketQuote::MissingUsage | BucketQuote::MissingRate => None,
        })
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

/// `day` with every attempt priced from its reported charge where it has one,
/// and its totals summed again where any attempt changed. Fails only when
/// the exact sums overflow.
pub(super) fn priced_from_charges(day: InspectionDay) -> anyhow::Result<InspectionDay> {
    Ok(match day {
        InspectionDay::Ready(mut view) => {
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
                // Other conversations have no stored totals here; their
                // figures are summed from these quotes when shown.
                for quote in others.requests.values_mut().flatten() {
                    reprice(quote);
                }
            }
            if changed {
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
            for bucket in &mut buckets {
                bucket.days = std::mem::take(&mut bucket.days)
                    .into_iter()
                    .map(priced_from_charges)
                    .collect::<anyhow::Result<_>>()?;
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
    if let [quote] = billable.as_slice() {
        let silent = prices_from_charge(&quote.attempt.provider);
        return Some(if reported_charge(quote).is_some() {
            format!("Price basis: the charge {provider} reported with the response (usage.cost)")
        } else if quote.all_buckets_priced.is_some() {
            if silent {
                format!(
                    "Price basis: estimate from published prices — {provider} reported no charge for this response"
                )
            } else {
                "Price basis: estimate from published prices".to_string()
            }
        } else {
            let why = if lacks_price(quote) {
                "no published price covers this response"
            } else {
                "the response did not report every token count its published price needs"
            };
            if silent {
                format!("Price basis: none — {provider} reported no charge, and {why}")
            } else {
                format!("Price basis: none — {why}")
            }
        });
    }
    let (mut charged, mut estimated, mut silent_estimated, mut none) = (0, 0, 0, 0);
    for quote in &billable {
        if reported_charge(quote).is_some() {
            charged += 1;
        } else if quote.all_buckets_priced.is_some() {
            estimated += 1;
            silent_estimated += usize::from(prices_from_charge(&quote.attempt.provider));
        } else {
            none += 1;
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
        parts.push(if silent_estimated > 0 {
            format!(
                "{} estimated from published prices ({provider} reported no charge for {silent_estimated})",
                attempts(estimated)
            )
        } else {
            format!("{} estimated from published prices", attempts(estimated))
        });
    }
    if none > 0 {
        parts.push(format!("{} with no price", attempts(none)));
    }
    Some(format!("Price basis: {}", parts.join("; ")))
}

#[cfg(test)]
#[path = "charge_tests.rs"]
mod tests;
