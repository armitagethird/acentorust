//! Accent table: which letters have accented variants, and in what order.

/// A letter that has accented variants in Portuguese.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Letter {
    A,
    C,
    E,
    I,
    O,
    U,
}

impl Letter {
    /// Maps a Windows virtual-key code (`'A'` = 0x41 …) to a letter with accents.
    pub fn from_vk(vk: u16) -> Option<Self> {
        match u8::try_from(vk).ok()? {
            b'A' => Some(Self::A),
            b'C' => Some(Self::C),
            b'E' => Some(Self::E),
            b'I' => Some(Self::I),
            b'O' => Some(Self::O),
            b'U' => Some(Self::U),
            _ => None,
        }
    }

    /// The virtual-key code of this letter.
    pub fn vk(self) -> u16 {
        u16::from(match self {
            Self::A => b'A',
            Self::C => b'C',
            Self::E => b'E',
            Self::I => b'I',
            Self::O => b'O',
            Self::U => b'U',
        })
    }

    /// Accented variants in display order: most frequent in Portuguese first, so the common
    /// ones need the fewest Space presses.
    pub fn variants(self, upper: bool) -> &'static [char] {
        match (self, upper) {
            (Self::A, false) => &['á', 'ã', 'â', 'à'],
            (Self::A, true) => &['Á', 'Ã', 'Â', 'À'],
            (Self::C, false) => &['ç'],
            (Self::C, true) => &['Ç'],
            (Self::E, false) => &['é', 'ê'],
            (Self::E, true) => &['É', 'Ê'],
            (Self::I, false) => &['í'],
            (Self::I, true) => &['Í'],
            (Self::O, false) => &['ó', 'õ', 'ô'],
            (Self::O, true) => &['Ó', 'Õ', 'Ô'],
            (Self::U, false) => &['ú'],
            (Self::U, true) => &['Ú'],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Letter; 6] = [
        Letter::A,
        Letter::C,
        Letter::E,
        Letter::I,
        Letter::O,
        Letter::U,
    ];

    #[test]
    fn from_vk_maps_only_accentable_letters() {
        assert_eq!(Letter::from_vk(0x41), Some(Letter::A));
        assert_eq!(Letter::from_vk(0x43), Some(Letter::C));
        assert_eq!(Letter::from_vk(0x55), Some(Letter::U));
        assert_eq!(Letter::from_vk(0x42), None); // B
        assert_eq!(Letter::from_vk(0x20), None); // Space
        assert_eq!(Letter::from_vk(0x61), None); // VK_NUMPAD1, not a lowercase 'a'
        assert_eq!(Letter::from_vk(0x141), None); // outside the u8 range
    }

    #[test]
    fn vk_round_trips() {
        for letter in ALL {
            assert_eq!(Letter::from_vk(letter.vk()), Some(letter));
        }
    }

    #[test]
    fn portuguese_variants_in_frequency_order() {
        assert_eq!(Letter::A.variants(false), ['á', 'ã', 'â', 'à']);
        assert_eq!(Letter::E.variants(false), ['é', 'ê']);
        assert_eq!(Letter::I.variants(false), ['í']);
        assert_eq!(Letter::O.variants(false), ['ó', 'õ', 'ô']);
        assert_eq!(Letter::U.variants(false), ['ú']);
        assert_eq!(Letter::C.variants(false), ['ç']);
    }

    #[test]
    fn uppercase_variants_mirror_lowercase() {
        for letter in ALL {
            let lower = letter.variants(false);
            let upper = letter.variants(true);
            assert_eq!(lower.len(), upper.len());
            for (lc, uc) in lower.iter().zip(upper) {
                assert_eq!(lc.to_uppercase().collect::<Vec<_>>(), vec![*uc]);
            }
        }
    }
}
