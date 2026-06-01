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
        }
    }

    /// The current engine state. Always present between turns.
    pub fn engine(&self) -> &HandState {
        self.engine.as_ref().expect("engine present between turns")
    }

    pub fn view(&self) -> GameState {
        to_presentation(self.engine(), &self.names)
    }

    /// Returns true if the key was consumed (engine state may have changed).
    pub fn handle_key(&mut self, key: KeyCode) -> bool {
        let engine = self.engine();
        let Some(actor) = engine.to_act else {
            return false;
        };
        // Only the hero (the human at the keyboard for pass-and-play) acts via keys.
        // In hot-seat, every active player IS the hero from their own perspective —
        // we accept input for whoever is to_act. This makes pass-and-play work.
        let _ = actor;

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
                // Minimum legal raise / bet.
                if engine.current_bet == 0 {
                    Some(Action::Bet {
                        to: engine.config.big_blind,
                    })
                } else {
                    Some(Action::Raise {
                        to: engine.current_bet + engine.min_raise,
                    })
                }
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
}
