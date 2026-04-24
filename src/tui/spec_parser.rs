use std::fs;

#[derive(Clone)]
pub struct Feature {
    pub name: String,
    pub status: FeatureStatus,
}

#[derive(Clone, PartialEq)]
pub enum FeatureStatus {
    NotStarted,
    InProgress,
    Completed,
}

/// Parse features from .harness/spec.md.
/// Looks for patterns like "Feature N: ..." or "### Feature ..." or numbered list items.
pub fn parse_features() -> Vec<Feature> {
    let spec_path = std::path::Path::new(".harness/spec.md");
    let content = match fs::read_to_string(spec_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut features = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        // Match "Feature N: description" or "### Feature N: description"
        if let Some(rest) = trimmed
            .strip_prefix("### Feature")
            .or_else(|| trimmed.strip_prefix("Feature"))
        {
            let name = rest.trim_start_matches(|c: char| {
                c.is_ascii_digit() || c == ':' || c == '.' || c == ' '
            });
            if !name.is_empty() {
                features.push(Feature {
                    name: name.trim().to_string(),
                    status: FeatureStatus::NotStarted,
                });
            }
        }
        // Match "- [ ] feature" or "- [x] feature" (checkbox lists)
        else if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
            features.push(Feature {
                name: rest.to_string(),
                status: FeatureStatus::NotStarted,
            });
        } else if let Some(rest) = trimmed
            .strip_prefix("- [x] ")
            .or_else(|| trimmed.strip_prefix("- [X] "))
        {
            features.push(Feature {
                name: rest.to_string(),
                status: FeatureStatus::Completed,
            });
        }
        // Match numbered items like "1. Feature description" under a features heading
        else if trimmed.len() > 2
            && trimmed.as_bytes()[0].is_ascii_digit()
            && trimmed.contains(". ")
            && let Some(rest) = trimmed.split_once(". ")
        {
            let text = rest.1.trim();
            if !text.is_empty()
                && text.len() < 100
                && text.chars().next().is_some_and(|c| c.is_uppercase())
            {
                features.push(Feature {
                    name: text.to_string(),
                    status: FeatureStatus::NotStarted,
                });
            }
        }
    }
    features
}

/// Update feature statuses based on builder output lines.
/// Detects file writes, git commits, or mentions of feature names.
pub fn update_feature_status(features: &mut [Feature], line: &str) {
    let lower = line.to_lowercase();
    for feature in features.iter_mut() {
        if feature.status == FeatureStatus::Completed {
            continue;
        }
        // Check if the line mentions keywords from the feature name
        let feature_lower = feature.name.to_lowercase();
        let words: Vec<&str> = feature_lower.split_whitespace().collect();
        // Need at least 2 significant words to match (or all if feature name is short)
        let threshold = if words.len() <= 2 { words.len() } else { 2 };
        let matched = words
            .iter()
            .filter(|w| w.len() > 2 && lower.contains(*w))
            .count();
        if matched >= threshold {
            feature.status = FeatureStatus::InProgress;
        }
    }
}
