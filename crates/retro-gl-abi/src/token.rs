//! Context/drawable tokens with an INCARNATION: a slot reused after destroy gets a new incarnation, so a token held
//! from the slot's previous life is refused ([`TokenError::Stale`]) instead of acting on someone else's object.
use alloc::vec::Vec;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Token {
    pub slot: u32,
    pub incarnation: u32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TokenError {
    /// The slot exists but this token is from an earlier life of it, or it was destroyed.
    Stale,
    /// No such slot.
    Unknown,
    /// Slot numbers are exhausted (named refusal; the table itself has no fixed capacity).
    Exhausted,
}

struct Slot {
    incarnation: u32,
    live: bool,
}

/// A growable token table (no hard cap).
#[derive(Default)]
pub struct TokenTable {
    slots: Vec<Slot>,
    free: Vec<u32>,
}

impl TokenTable {
    pub fn create(&mut self) -> Result<Token, TokenError> {
        if let Some(i) = self.free.pop() {
            let s = &mut self.slots[i as usize];
            s.live = true;
            return Ok(Token { slot: i, incarnation: s.incarnation });
        }
        let slot = u32::try_from(self.slots.len()).map_err(|_| TokenError::Exhausted)?;
        self.slots.push(Slot { incarnation: 1, live: true });
        Ok(Token { slot, incarnation: 1 })
    }

    /// `Ok(())` only for a live token of the slot's CURRENT incarnation.
    pub fn check(&self, t: Token) -> Result<(), TokenError> {
        let s = self.slots.get(t.slot as usize).ok_or(TokenError::Unknown)?;
        if s.live && s.incarnation == t.incarnation {
            Ok(())
        } else {
            Err(TokenError::Stale)
        }
    }

    /// Destroy: the slot's incarnation advances; a slot whose incarnation would wrap is retired, never reused.
    pub fn destroy(&mut self, t: Token) -> Result<(), TokenError> {
        self.check(t)?;
        let s = &mut self.slots[t.slot as usize];
        s.live = false;
        if let Some(n) = s.incarnation.checked_add(1) {
            s.incarnation = n;
            self.free.push(t.slot);
        }
        Ok(())
    }
}
