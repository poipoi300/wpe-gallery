// Explicit lifecycle shared by empty-state copy and async verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verification {
    Checking { completed: usize, total: usize },
    Ready,
    Failed(String),
}
impl Verification {
    pub fn progress(&self) -> Option<String> {
        match self {
            Self::Checking { completed, total } => Some(format!(
                "Verifying {total} wallpapers… {completed} of {total} checked"
            )),
            _ => None,
        }
    }
    pub fn empty_message(&self, has_items: bool) -> String {
        match self {
            Self::Checking { .. } => self.progress().unwrap(),
            Self::Failed(error) => error.clone(),
            Self::Ready if !has_items => "No wallpapers found. Download Workshop items through Steam, then reopen.".into(),
            Self::Ready => "No verified safe wallpapers. Add reviewed entries to whitelist.json and reload reviews, or turn Safe only off to inspect unreviewed items.".into(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completion_and_errors_are_distinct_from_empty_results() {
        let checking = Verification::Checking {
            completed: 0,
            total: 464,
        };
        assert!(checking.empty_message(true).contains("Verifying 464"));
        let error = Verification::Failed(
            "Whitelist /fixture/whitelist.json: expected value at line 1 column 1".into(),
        );
        assert!(
            error
                .empty_message(true)
                .contains("/fixture/whitelist.json")
        );
        assert!(!error.empty_message(true).contains("No verified safe"));
        assert!(
            Verification::Ready
                .empty_message(true)
                .starts_with("No verified safe")
        );
    }
}
