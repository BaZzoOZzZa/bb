macro_rules! wire_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $($variant:ident => $value:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            Hash,
            Default,
            serde::Serialize,
            serde::Deserialize,
        )]
        $vis enum $name {
            $(
                #[serde(rename = $value)]
                $variant,
            )+
            #[default]
            #[serde(other, rename = "unknown")]
            Unknown,
        }

        impl $name {
            pub const KNOWN: &'static [Self] = &[$(Self::$variant),+];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value,)+
                    Self::Unknown => "unknown",
                }
            }

            pub const fn is_known(self) -> bool {
                !matches!(self, Self::Unknown)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

macro_rules! default_unknown {
    ($($name:ty),+ $(,)?) => {
        $(
            impl Default for $name {
                fn default() -> Self {
                    Self::Unknown(serde_json::Value::Null)
                }
            }
        )+
    };
}
