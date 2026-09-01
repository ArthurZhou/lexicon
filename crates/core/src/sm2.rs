//! Anki-style SM2 scheduling. Pure, deterministic, unit-testable.

use crate::model::{Card, ReviewStatus};
use chrono::NaiveDateTime;

const DEFAULT_EASE: f64 = 2.5;
const MIN_EASE: f64 = 1.3;

#[derive(Debug, Clone, Copy)]
pub struct Rating {
    /// 0 = Again, 1 = Hard, 2 = Good, 3 = Easy
    pub grade: u8,
    pub reviewed_on: NaiveDateTime,
}

impl Rating {
    pub fn new(grade: u8, reviewed_on: NaiveDateTime) -> Self {
        Self { grade, reviewed_on }
    }
}

pub fn now() -> NaiveDateTime {
    chrono::Local::now().naive_local()
}

fn plus_days(on: NaiveDateTime, days: f64) -> NaiveDateTime {
    on + chrono::Duration::days(days.ceil() as i64)
}

/// Apply one review and return the updated card. The prompt difficulty is
/// auto-adjusted to match how familiar the learner is with the card:
///  - forgot / hard  -> step down (more help next time)
///  - easy           -> step up (less help), but only after a few reps so a
///                      single lucky guess doesn't jump straight to hard
pub fn apply_review(card: &mut Card, r: Rating) {
    let g = r.grade.min(3);
    match g {
        0 => {
            again(card, r.reviewed_on);
            card.difficulty.step_down();
        }
        1 => {
            hard(card, r.reviewed_on);
            card.difficulty.step_down();
        }
        2 => good(card, r.reviewed_on),
        _ => {
            easy(card, r.reviewed_on);
            if card.reps >= 2 {
                card.difficulty.step_up();
            }
        }
    }
}

fn again(card: &mut Card, on: NaiveDateTime) {
    card.lapses += 1;
    card.ease = (card.ease - 0.20).max(MIN_EASE);
    card.reps = 0;
    card.status = match card.status {
        ReviewStatus::Review => ReviewStatus::Relearning,
        s => s,
    };
    // relearn in 1 minute (represented as due right now)
    card.interval_days = 0.0;
    // NOTE: due must stay Some(..) — the daily web queue only picks up cards
    // with a non-NULL due, so None here would make lapsed cards disappear
    // from the review queue forever.
    card.due = Some(on);
}

fn hard(card: &mut Card, on: NaiveDateTime) {
    card.reps += 1;
    card.interval_days = match card.status {
        ReviewStatus::Review => (card.interval_days * 1.2).max(1.0),
        _ => 1.0,
    };
    card.status = ReviewStatus::Review;
    card.due = Some(plus_days(on, card.interval_days));
}

fn good(card: &mut Card, on: NaiveDateTime) {
    card.reps += 1;
    card.interval_days = match card.reps {
        1 => 1.0,
        2 => 6.0,
        _ => (card.interval_days * card.ease).max(1.0),
    };
    card.status = ReviewStatus::Review;
    card.ease = (card.ease + 0.0).max(MIN_EASE);
    card.due = Some(plus_days(on, card.interval_days));
}

fn easy(card: &mut Card, on: NaiveDateTime) {
    card.reps += 1;
    card.interval_days = match card.reps {
        1 => 4.0,
        2 => 7.0,
        _ => (card.interval_days * card.ease * 1.15).max(1.0),
    };
    card.status = ReviewStatus::Review;
    card.ease = (card.ease + 0.15).max(MIN_EASE);
    card.due = Some(plus_days(on, card.interval_days));
}

pub fn new_card(headword: String, sense: usize) -> Card {
    Card {
        id: 0,
        headword,
        sense,
        card_type: crate::model::CardType::Word,
        difficulty: crate::model::Difficulty::Easy,
        source: String::new(),
        phrase: String::new(),
        due: None,
        interval_days: 0.0,
        ease: DEFAULT_EASE,
        reps: 0,
        lapses: 0,
        status: ReviewStatus::New,
        created_at: chrono::Local::now().naive_local(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    #[test]
    fn new_card_first_good_is_one_day() {
        let mut c = new_card("apple".into(), 0);
        apply_review(&mut c, Rating::new(2, day("2026-08-29 09:00:00")));
        assert_eq!(c.interval_days, 1.0);
        assert_eq!(c.status, ReviewStatus::Review);
        assert_eq!(c.due, Some(day("2026-08-30 09:00:00")));
    }

    #[test]
    fn good_second_is_six_days() {
        let mut c = new_card("apple".into(), 0);
        apply_review(&mut c, Rating::new(2, day("2026-08-29 09:00:00")));
        apply_review(&mut c, Rating::new(2, day("2026-08-30 09:00:00")));
        assert_eq!(c.interval_days, 6.0);
    }

    #[test]
    fn again_resets_interval() {
        let mut c = new_card("apple".into(), 0);
        apply_review(&mut c, Rating::new(2, day("2026-08-29 09:00:00")));
        apply_review(&mut c, Rating::new(0, day("2026-08-30 09:00:00")));
        assert_eq!(c.reps, 0);
        assert!(c.interval_days < 1.0);
        assert_eq!(c.status, ReviewStatus::Relearning);
    }

    #[test]
    fn lapsed_card_stays_due_now() {
        // Regression: lapsed cards used to get due=None, which made them
        // disappear from the daily review queue (it only picks due IS NOT NULL).
        let mut c = new_card("apple".into(), 0);
        apply_review(&mut c, Rating::new(2, day("2026-08-29 09:00:00")));
        apply_review(&mut c, Rating::new(0, day("2026-08-30 09:00:00")));
        assert_eq!(c.due, Some(day("2026-08-30 09:00:00")));
    }

    #[test]
    fn difficulty_matches_familiarity() {
        use crate::model::Difficulty;
        let mut c = new_card("apple".into(), 0);
        // forgetting immediately drops the prompt difficulty
        apply_review(&mut c, Rating::new(2, day("2026-08-29 09:00:00")));
        apply_review(&mut c, Rating::new(0, day("2026-08-30 09:00:00")));
        assert_eq!(c.difficulty, Difficulty::Easy);
        // a couple of easy grades ramp it back up
        apply_review(&mut c, Rating::new(3, day("2026-08-31 09:00:00")));
        assert_eq!(c.difficulty, Difficulty::Easy); // first easy after lapse: no bump yet
        apply_review(&mut c, Rating::new(3, day("2026-09-03 09:00:00")));
        assert_eq!(c.difficulty, Difficulty::Medium);
        apply_review(&mut c, Rating::new(3, day("2026-09-10 09:00:00")));
        assert_eq!(c.difficulty, Difficulty::Hard);
        // forgetting again eases the prompt back off
        apply_review(&mut c, Rating::new(0, day("2026-09-24 09:00:00")));
        assert_eq!(c.difficulty, Difficulty::Medium);
    }

    #[test]
    fn ease_floor() {
        let mut c = new_card("apple".into(), 0);
        for _ in 0..10 {
            apply_review(&mut c, Rating::new(0, day("2026-08-29 09:00:00")));
        }
        assert!(c.ease >= MIN_EASE);
    }
}
