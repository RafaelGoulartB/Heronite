//! The choices of Settings → Appearance that are one value out of a list.
//! Each is stored by its `as_str` name; an unknown name gives the default,
//! which is always today's look.

/// A fieldless enum stored by name, with `ALL`, `DEFAULT`, `as_str` and `parse`.
macro_rules! choice {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident = $key:literal),+ $(,)? } default $default:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];
            pub const DEFAULT: $name = $name::$default;

            pub fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $key),+
                }
            }

            pub fn parse(s: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|c| c.as_str() == s)
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::DEFAULT
            }
        }
    };
}

choice! {
    /// Colour of the selection, links, active controls and sliders.
    Accent {
        Ice = "ice",
        Mint = "mint",
        Amber = "amber",
        Rose = "rose",
        Lavender = "lavender",
        /// Taken from the selected game's art (the app falls back to `Ice`
        /// while there is none).
        Game = "game",
    } default Ice
}

impl Accent {
    /// The colour of a preset; `None` for [`Accent::Game`].
    pub fn rgb(self) -> Option<(u8, u8, u8)> {
        Some(match self {
            Self::Ice => (0xa5, 0xdc, 0xeb),
            Self::Mint => (0x7e, 0xe0, 0xb5),
            Self::Amber => (0xff, 0xc4, 0x6b),
            Self::Rose => (0xff, 0x8f, 0xa3),
            Self::Lavender => (0xb9, 0xa6, 0xff),
            Self::Game => return None,
        })
    }
}

impl Accent {
    /// The colour of a preset on the light theme: the same hue, dark enough
    /// for text on the light panels. `None` for [`Accent::Game`].
    pub fn rgb_light(self) -> Option<(u8, u8, u8)> {
        Some(match self {
            Self::Ice => (0x1b, 0x6c, 0x83),
            Self::Mint => (0x14, 0x73, 0x4a),
            Self::Amber => (0x92, 0x56, 0x00),
            Self::Rose => (0xb4, 0x34, 0x54),
            Self::Lavender => (0x66, 0x4b, 0xd5),
            Self::Game => return None,
        })
    }
}

choice! {
    /// Light or dark panels and text.
    Theme {
        Dark = "dark",
        Light = "light",
        /// The desktop's light or dark style.
        System = "system",
    } default Dark
}

choice! {
    /// Font families of the interface.
    FontChoice {
        /// Cabin for text, Rubik for titles.
        Default = "default",
        /// Rubik for everything.
        Rubik = "rubik",
        /// The desktop's interface font.
        System = "system",
    } default Default
}

choice! {
    /// Spacing between the covers of the list and the grid.
    Density {
        Comfortable = "comfortable",
        Compact = "compact",
    } default Comfortable
}

choice! {
    /// Corners of the covers in the list and the grid.
    Corners {
        Square = "square",
        Soft = "soft",
        Round = "round",
    } default Soft
}

impl Corners {
    /// Radius of the cover, in logical px.
    pub fn radius(self) -> f32 {
        match self {
            Self::Square => 2.,
            Self::Soft => 6.,
            Self::Round => 12.,
        }
    }
}

choice! {
    /// The line under each cover.
    CoverLabel {
        /// The playtime, or the value the list is sorted by.
        Auto = "auto",
        Playtime = "playtime",
        LastPlayed = "lastPlayed",
        Status = "status",
        Title = "title",
        Nothing = "none",
    } default Auto
}

choice! {
    /// What a cover does under the pointer (the play bar shows either way).
    CoverHover {
        Nothing = "none",
        /// Rises a few px with a deeper shadow.
        Lift = "lift",
        /// Brightens, with a ring in the accent colour.
        Glow = "glow",
    } default Nothing
}

choice! {
    /// The picture behind the Collection.
    BackgroundSource {
        /// The selected game's hero art.
        GameArt = "art",
        /// One of the player's screenshots of the selected game (the art when
        /// there are none).
        Screenshot = "screenshot",
        /// The selected game's cover, blurred.
        Cover = "cover",
        /// No picture: the panels' colour.
        Solid = "solid",
        /// A picture the user picked (copied to `AppPaths::custom_background`).
        Custom = "custom",
    } default GameArt
}

choice! {
    /// Fades and transitions.
    Motion {
        Full = "full",
        /// Only the background's cross-fade.
        Reduced = "reduced",
        Off = "off",
    } default Full
}

choice! {
    /// The Collection view the app opens in.
    StartView {
        /// The one used last (`ui-state.json`'s `gridView`).
        Last = "last",
        Focus = "focus",
        Grid = "grid",
    } default Last
}

choice! {
    /// The blocks of a game's details (Focus panel and grid drawer).
    Section {
        Screenshots = "screenshots",
        Overview = "overview",
        Details = "details",
        Timeline = "timeline",
        Related = "related",
    } default Overview
}

/// Order and visibility of the [`Section`]s. In the Focus panel's two
/// columns "Details" stays in the side column; the others keep this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sections {
    order: [Section; 5],
    /// Bit `1 << index in Section::ALL` set = hidden.
    hidden: u8,
}

impl Sections {
    pub const DEFAULT: Sections =
        Sections { order: [Section::Screenshots, Section::Overview, Section::Details, Section::Timeline, Section::Related], hidden: 0 };

    fn bit(s: Section) -> u8 {
        1 << Section::ALL.iter().position(|x| *x == s).unwrap_or(0)
    }

    pub fn order(&self) -> [Section; 5] {
        self.order
    }

    pub fn shown(&self, s: Section) -> bool {
        self.hidden & Self::bit(s) == 0
    }

    /// The shown sections, in order.
    pub fn visible(&self) -> impl Iterator<Item = Section> + '_ {
        self.order.into_iter().filter(|s| self.shown(*s))
    }

    pub fn toggled(mut self, s: Section) -> Self {
        self.hidden ^= Self::bit(s);
        self
    }

    /// `s` moved `delta` places (clamped to the ends).
    pub fn moved(mut self, s: Section, delta: i32) -> Self {
        if let Some(i) = self.order.iter().position(|x| *x == s) {
            let j = (i as i32 + delta).clamp(0, self.order.len() as i32 - 1) as usize;
            let item = self.order[i];
            if i < j {
                self.order.copy_within(i + 1..=j, i);
            } else {
                self.order.copy_within(j..i, j + 1);
            }
            self.order[j] = item;
        }
        self
    }

    /// From stored names: unknown names are dropped, missing sections are
    /// added in their default place.
    pub fn from_names<'a>(order: impl IntoIterator<Item = &'a str>, hidden: impl IntoIterator<Item = &'a str>) -> Self {
        let mut seen: Vec<Section> = Vec::with_capacity(5);
        for s in order.into_iter().filter_map(Section::parse) {
            if !seen.contains(&s) {
                seen.push(s);
            }
        }
        for s in Self::DEFAULT.order {
            if !seen.contains(&s) {
                seen.push(s);
            }
        }
        let mut out = Self { order: Self::DEFAULT.order, hidden: 0 };
        out.order.copy_from_slice(&seen[..5]);
        for s in hidden.into_iter().filter_map(Section::parse) {
            out.hidden |= Self::bit(s);
        }
        out
    }

    pub fn hidden_names(&self) -> Vec<&'static str> {
        Section::ALL.iter().filter(|s| !self.shown(**s)).map(|s| s.as_str()).collect()
    }
}

impl Default for Sections {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for a in Accent::ALL {
            assert_eq!(Accent::parse(a.as_str()), Some(*a));
        }
        for s in Section::ALL {
            assert_eq!(Section::parse(s.as_str()), Some(*s));
        }
        for t in Theme::ALL {
            assert_eq!(Theme::parse(t.as_str()), Some(*t));
        }
        assert_eq!(CoverLabel::parse("lastPlayed"), Some(CoverLabel::LastPlayed));
        assert_eq!(Motion::parse("slow"), None);
        assert_eq!(Theme::parse("auto"), None);
    }

    #[test]
    fn sections_move_and_hide() {
        let s = Sections::DEFAULT.moved(Section::Related, -10);
        assert_eq!(s.order()[0], Section::Related);
        assert_eq!(s.order()[1], Section::Screenshots);
        let s = s.moved(Section::Related, 2);
        assert_eq!(s.order()[..3], [Section::Screenshots, Section::Overview, Section::Related]);
        let s = s.toggled(Section::Timeline);
        assert!(!s.shown(Section::Timeline));
        assert_eq!(s.visible().count(), 4);
        assert_eq!(s.hidden_names(), ["timeline"]);
        assert!(s.toggled(Section::Timeline).shown(Section::Timeline));
    }

    #[test]
    fn sections_from_partial_names() {
        let s = Sections::from_names(["related", "bogus", "related", "overview"], ["details", "x"]);
        assert_eq!(s.order(), [Section::Related, Section::Overview, Section::Screenshots, Section::Details, Section::Timeline]);
        assert!(!s.shown(Section::Details));
        assert_eq!(Sections::from_names([], []), Sections::DEFAULT);
    }
}
