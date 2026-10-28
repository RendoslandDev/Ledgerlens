macro_rules! str_enum {
    ($(#[$m:meta])* $name:ident { $($var:ident => $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[cfg_attr(feature = "specta", derive(specta::Type))]
        pub enum $name { $( #[serde(rename = $s)] $var ),+ }
        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool { self.as_str() == *other }
        }
        impl $name {
            pub const fn as_str(self) -> &'static str { match self { $( Self::$var => $s ),+ } }
        }
        impl FromStr for $name {
            type Err = crate::error::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s { $( $s => Ok(Self::$var), )+
                    other => Err(crate::error::Error::Invalid(format!("unknown {}: {other}", stringify!($name)))) }
            }
        }
    };
}

