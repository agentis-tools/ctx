//! Near-duplicate function detection command.
//!
//! `ctx duplicates` compares MinHash fingerprints (built during `ctx index`)
//! of every indexed function/method and reports pairs whose normalized token
//! shingles have a Jaccard similarity at or above the threshold. See
//! `src/fingerprint.rs` for the algorithm.

use std::env;

use ctx::error::Result;
use ctx::exit::Outcome;
use ctx::fingerprint::{find_near_duplicates, DuplicatePair, MIN_THRESHOLD, SHINGLE_K};
use ctx::gitutil;
use ctx::index;
use ctx::json::{emit, SymbolRef};

/// Detect structurally similar functions via MinHash near-duplicate search.
pub fn run_duplicates(
    threshold: f64,
    min_tokens: i64,
    against: Option<&str>,
    json: bool,
    fail_on_found: bool,
) -> Result<Outcome> {
    if !(0.0..=1.0).contains(&threshold) {
        return Err(format!(
            "--threshold must be a Jaccard similarity between 0.0 and 1.0 (got {})",
            threshold
        )
        .into());
    }
    let threshold = if threshold < MIN_THRESHOLD {
        eprintln!(
            "Warning: --threshold {} is below {}; clamping to {} (LSH candidate \
             detection is unreliable below that)",
            threshold, MIN_THRESHOLD, MIN_THRESHOLD
        );
        MIN_THRESHOLD
    } else {
        threshold
    };

    // Resolve the changed-file set first so bad references fail fast.
    let changed = match against {
        Some(reference) => Some(gitutil::changed_files_against(reference)?),
        None => None,
    };

    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    let pairs = find_near_duplicates(&db, threshold, min_tokens, changed.as_ref())?;

    if json {
        let json_pairs: Vec<_> = pairs
            .iter()
            .map(|p| {
                serde_json::json!({
                    "a": SymbolRef::from(&p.a).to_value(),
                    "b": SymbolRef::from(&p.b).to_value(),
                    "similarity": p.similarity,
                    "token_count_a": p.token_count_a,
                    "token_count_b": p.token_count_b,
                })
            })
            .collect();
        emit(
            "duplicates",
            serde_json::json!({
                "threshold": threshold,
                "min_tokens": min_tokens,
                "against": against,
                // Every supported language is fingerprinted (Solidity via the
                // solang-parser lexer), so nothing is skipped.
                "skipped_languages": [],
                "pairs": json_pairs,
            }),
        )?;
    } else {
        print_human(&pairs, threshold, min_tokens, against);
    }

    Ok(outcome(fail_on_found, pairs.len()))
}

/// Map the pair count to the exit outcome: `--fail-on-found` turns any
/// reported pair into exit code 1; otherwise the command is informational.
fn outcome(fail_on_found: bool, pair_count: usize) -> Outcome {
    if fail_on_found && pair_count > 0 {
        Outcome::Findings
    } else {
        Outcome::Clean
    }
}

fn print_human(pairs: &[DuplicatePair], threshold: f64, min_tokens: i64, against: Option<&str>) {
    let scope = match against {
        Some(reference) => format!(", changed vs {}", reference),
        None => String::new(),
    };

    if pairs.is_empty() {
        println!(
            "No near-duplicate functions found (Jaccard >= {:.2}, >= {} tokens{}).",
            threshold, min_tokens, scope
        );
        return;
    }

    println!(
        "Near-duplicate functions (Jaccard similarity of {}-token shingles >= {:.2}, >= {} tokens{})",
        SHINGLE_K, threshold, min_tokens, scope
    );
    println!("{}", "=".repeat(100));

    for (i, pair) in pairs.iter().enumerate() {
        println!("\n{}. similarity {:.3}", i + 1, pair.similarity);
        println!(
            "   {}:{} {} ({} tokens)",
            pair.a.file_path, pair.a.line_start, pair.a.name, pair.token_count_a
        );
        println!(
            "   {}:{} {} ({} tokens)",
            pair.b.file_path, pair.b.line_start, pair.b.name, pair.token_count_b
        );
    }

    println!("\n{}", "-".repeat(100));
    println!("Found {} near-duplicate pair(s).", pairs.len());
    println!(
        "Note: idiomatic boilerplate can look structurally similar; raise --min-tokens to filter short functions."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fail_on_found_outcome_mapping() {
        // --fail-on-found: Findings only when pairs exist.
        assert_eq!(outcome(true, 3), Outcome::Findings);
        assert_eq!(outcome(true, 0), Outcome::Clean);
        // Default mode is informational regardless of pairs.
        assert_eq!(outcome(false, 3), Outcome::Clean);
        assert_eq!(outcome(false, 0), Outcome::Clean);
    }
}
