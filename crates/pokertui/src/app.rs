use crossterm::event::KeyCode;
use poker_core::holdem::{Action, HandConfig, HandState, PlayerId, apply, legal_actions};

use crate::adapter::{NameRegistry, to_presentation};
use crate::state::GameState;

pub struct App {
    // `Option` so a turn can move the engine out (apply consumes it by value)
    // and move the result back without a placeholder. It is always `Some`
    // outside of `handle_key`; access it through `engine()`.
    engine: Option<HandState>,
    pub names: NameRegistry,
    /// Currently selected raise/bet to-level. `None` = untouched → use min.
    /// Reset to `None` after every committed action.
    raise_to: Option<u64>,
}

impl App {
    pub fn new_demo_hand() -> Self {
        // Seed from the wall clock so each launch deals a different hand.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0xC0FFEE);
        let cfg = HandConfig {
            num_players: 6,
            small_blind: 50,
            big_blind: 100,
            dealer: PlayerId(0),
            seed,
        };
        let engine = HandState::new_hand(cfg, vec![10_000; 6]);
        let names = NameRegistry::demo_six();
        Self {
            engine: Some(engine),
            names,
            raise_to: None,
        }
    }

    /// The current engine state. Always present between turns.
    pub fn engine(&self) -> &HandState {
        self.engine.as_ref().expect("engine present between turns")
    }

    /// `(min, max)` raise/bet *to-level* for the player currently to act, or
    /// `None` when no legal raise/bet exists (no actor, or stack too small to
    /// make a full min-raise — only an all-in would be legal there).
    fn raise_bounds(engine: &HandState) -> Option<(u64, u64)> {
        let actor = engine.to_act?;
        let min = if engine.current_bet == 0 {
            engine.config.big_blind
        } else {
            engine.current_bet + engine.min_raise
        };
        // Going all-in defines the ceiling for the to-level.
        let max = engine.round_bet[actor.0] + engine.stacks[actor.0];
        (max >= min).then_some((min, max))
    }

    pub fn view(&self) -> GameState {
        let mut gs = to_presentation(self.engine(), &self.names);
        gs.phase.raise_to = Self::raise_bounds(self.engine())
            .map(|(min, max)| self.raise_to.unwrap_or(min).clamp(min, max));
        gs
    }

    /// Returns true if the key was consumed (engine state may have changed).
    pub fn handle_key(&mut self, key: KeyCode) -> bool {
        if self.engine().to_act.is_none() {
            return false;
        }

        // Bet-size selection: UI-only, mutates the selection, no engine change.
        if matches!(key, KeyCode::Up | KeyCode::Down) {
            if let Some((min, max)) = Self::raise_bounds(self.engine()) {
                let step = self.engine().config.small_blind;
                let cur = self.raise_to.unwrap_or(min);
                let next = if matches!(key, KeyCode::Up) {
                    cur.saturating_add(step).min(max)
                } else {
                    cur.saturating_sub(step).max(min)
                };
                self.raise_to = Some(next);
            }
            // Selection changed but the engine did not; the loop repaints every tick.
            return false;
        }

        let engine = self.engine();
        // Only the hero (the human at the keyboard for pass-and-play) acts via keys.
        // In hot-seat, every active player IS the hero from their own perspective —
        // we accept input for whoever is to_act. This makes pass-and-play work.

        let action = match key {
            KeyCode::Char('f') | KeyCode::Char('F') => Some(Action::Fold),
            KeyCode::Char('c') | KeyCode::Char('C') => {
                if legal_actions(engine).contains(&Action::Check) {
                    Some(Action::Check)
                } else {
                    Some(Action::Call)
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                Self::raise_bounds(engine).map(|(min, max)| {
                    let to = self.raise_to.unwrap_or(min).clamp(min, max);
                    if engine.current_bet == 0 {
                        Action::Bet { to }
                    } else {
                        Action::Raise { to }
                    }
                })
            }
            KeyCode::Char('a') | KeyCode::Char('A') => Some(Action::AllIn),
            _ => None,
        };

        let Some(action) = action else {
            return false;
        };
        // Move the engine out, apply, and move the result (or the unchanged
        // state on an illegal move) back. No allocation, no placeholder.
        let taken = self.engine.take().expect("engine present between turns");
        match apply(taken, action) {
            Ok(next) => {
                self.engine = Some(next);
                self.raise_to = None;
                true
            }
            Err((restored, _)) => {
                self.engine = Some(restored);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressing_f_folds_current_actor() {
        let mut app = App::new_demo_hand();
        let actor_before = app.engine().to_act.unwrap();
        app.handle_key(KeyCode::Char('f'));
        assert!(app.engine().folded[actor_before.0]);
    }

    #[test]
    fn pressing_c_calls_when_facing_bet() {
        let mut app = App::new_demo_hand();
        let actor = app.engine().to_act.unwrap();
        app.handle_key(KeyCode::Char('c'));
        assert_eq!(app.engine().round_bet[actor.0], 100);
    }

    #[test]
    fn pressing_r_raises_to_min() {
        let mut app = App::new_demo_hand();
        app.handle_key(KeyCode::Char('r'));
        assert_eq!(app.engine().current_bet, 200);
    }

    #[test]
    fn up_arrow_raises_selection_by_one_small_blind() {
        let mut app = App::new_demo_hand();
        // Preflop, facing BB: min raise-to is 200, small blind is 50.
        app.handle_key(KeyCode::Up);
        assert_eq!(app.view().phase.raise_to, Some(250));
        app.handle_key(KeyCode::Up);
        assert_eq!(app.view().phase.raise_to, Some(300));
    }

    #[test]
    fn down_arrow_clamps_at_min() {
        let mut app = App::new_demo_hand();
        // Already at min (200); stepping down must not go below it.
        app.handle_key(KeyCode::Down);
        assert_eq!(app.view().phase.raise_to, Some(200));
    }

    #[test]
    fn up_arrow_clamps_at_all_in() {
        let mut app = App::new_demo_hand();
        for _ in 0..1000 {
            app.handle_key(KeyCode::Up);
        }
        let actor = app.engine().to_act.unwrap();
        let max = app.engine().round_bet[actor.0] + app.engine().stacks[actor.0];
        assert_eq!(app.view().phase.raise_to, Some(max));
    }

    #[test]
    fn r_commits_at_selected_amount() {
        let mut app = App::new_demo_hand();
        app.handle_key(KeyCode::Up); // 250
        app.handle_key(KeyCode::Up); // 300
        app.handle_key(KeyCode::Char('r'));
        assert_eq!(app.engine().current_bet, 300);
    }

    #[test]
    fn selection_resets_after_a_committed_action() {
        let mut app = App::new_demo_hand();
        app.handle_key(KeyCode::Up); // 250
        app.handle_key(KeyCode::Char('r')); // commit raise to 250
        let min = app.engine().current_bet + app.engine().min_raise;
        assert_eq!(app.view().phase.raise_to, Some(min));
    }
}
