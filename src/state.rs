//! Logical destinations are independent of the currently rendered geometry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IslandState {
    #[default]
    Idle,
    Compact,
    Expanded,
    Volume,
    Media,
    Notification,
}

#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub width: f64,
    pub height: f64,
    pub radius: f64,
}

impl IslandState {
    pub fn geometry(self) -> Geometry {
        match self {
            Self::Idle => Geometry {
                width: 126.0,
                height: 32.0,
                radius: 16.0,
            },
            Self::Compact => Geometry {
                width: 220.0,
                height: 48.0,
                radius: 24.0,
            },
            Self::Expanded | Self::Media | Self::Volume | Self::Notification => Geometry {
                width: 360.0,
                height: 160.0,
                radius: 34.0,
            },
        }
    }
}
