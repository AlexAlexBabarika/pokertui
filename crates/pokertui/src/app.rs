use crossterm::event::KeyCode;
use poker_core::holdem::{Action, HandConfig, HandState, PlayerId, apply, legal_actions};

use crate::adapter::{NameRegistry, to_presentation};
use crate::state::GameState;

pub struct App {
    pub engine: HandState,
    pub names: NameRegistry,
}

impl App {
    pub fn new_demo_hand() -> Self {
        let cfg = HandConfig {
            num_players: 6,
            small_blind: 50,
            big_blind: 100,
            dealer: PlayerId(0),
            seed: 0xC0FFEE,
        };
        let engine = HandState::new_hand(cfg, vec![10_000; 6]);
        let names = NameRegistry::demo_six();
        Self { engine, names }
    }

    pub fn view(&self) -> GameState {
        to_presentation(&self.engine, &self.names)
    }

    /// Returns true if the key was consumed (engine state may have changed).
    pub fn handle_key(&mut self, key: KeyCode) -> bool {
        let Some(actor) = self.engine.to_act else {
            return false;
        };
        // Only the hero (the human at the keyboard for pass-and-play) acts via keys.
        // In hot-seat, every active player IS the hero from their own perspective —
        // we accept input for whoever is to_act. This makes pass-and-play work.
        let _ = actor;

        let action = match key {
            KeyCode::Char('f') | KeyCode::Char('F') => Some(Action::Fold),
            KeyCode::Char('c') | KeyCode::Char('C') => {
                if legal_actions(&self.engine).contains(&Action::Check) {
                    Some(Action::Check)
                } else {
                    Some(Action::Call)
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                // Minimum legal raise / bet.
                if self.engine.current_bet == 0 {
                    Some(Action::Bet {
                        to: self.engine.config.big_blind,
                    })
                } else {
                    Some(Action::Raise {
                        to: self.engine.current_bet + self.engine.min_raise,
                    })
                }
            }
            KeyCode::Char('a') | KeyCode::Char('A') => Some(Action::AllIn),
            _ => None,
        };

        let Some(action) = action else {
            return false;
        };
        // Replace engine state in place, ignoring illegal moves.
        let taken = std::mem::replace(&mut self.engine, dummy_engine());
        match apply(taken, action) {
            Ok(next) => {
                self.engine = next;
                true
            }
            Err((restored, _)) => {
                self.engine = restored;
                false
            }
        }
    }
}

fn dummy_engine() -> HandState {
    // Placeholder used only between `replace` and `match` above. Never observed
    // because the match arms re-assign immediately.
    HandState::new_hand(
        HandConfig {
            num_players: 2,
            small_blind: 1,
            big_blind: 2,
            dealer: PlayerId(0),
            seed: 0,
        },
        vec![10, 10],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressing_f_folds_current_actor() {
        let mut app = App::new_demo_hand();
        let actor_before = app.engine.to_act.unwrap();
        app.handle_key(KeyCode::Char('f'));
        assert!(app.engine.folded[actor_before.0]);
    }

    #[test]
    fn pressing_c_calls_when_facing_bet() {
        let mut app = App::new_demo_hand();
        let actor = app.engine.to_act.unwrap();
        app.handle_key(KeyCode::Char('c'));
        assert_eq!(app.engine.round_bet[actor.0], 100);
    }

    #[test]
    fn pressing_r_raises_to_min() {
        let mut app = App::new_demo_hand();
        app.handle_key(KeyCode::Char('r'));
        assert_eq!(app.engine.current_bet, 200);
    }
}
