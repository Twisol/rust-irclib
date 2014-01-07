#[crate_id="github.com/kballard/rust-irclib#irc:0.1"];
#[crate_type="rlib"];

//! Library for communicating with IRC servers

#[allow(dead_code)];
#[feature(macro_rules)]; // for tests

use std::vec;

pub mod conn;

#[deriving(Clone)]
pub struct User {
    priv raw: ~[u8],
    priv nicklen: uint,
    priv user: Option<(uint, uint)>,
    priv host: Option<(uint, uint)>
}

impl User {
    pub fn parse<V: CopyableVector<u8>>(v: V) -> User {
        let v = v.into_owned();

        let (mut bangloc, mut atloc) = (None, None);
        for (i, &b) in v.iter().enumerate() {
            if bangloc.is_none() && b == '!' as u8 {
                bangloc = Some(i);
            } else if b == '@' as u8 {
                atloc = Some(i);
                break;
            }
        }
        let nicklen = bangloc.or(atloc).unwrap_or(v.len());
        let user = bangloc.map(|i| (i+1, atloc.unwrap_or(v.len())));
        let host = atloc.map(|i| (i+1, v.len()));
        User{
            raw: v,
            nicklen: nicklen,
            user: user,
            host: host
        }
    }

    pub fn new(nick: &[u8], user: Option<&[u8]>, host: Option<&[u8]>) -> User {
        let cap = nick.len() + user.map_default(0, |v| v.len()+1) +
                  host.map_default(0, |v| v.len()+1);
        let mut raw = vec::with_capacity(cap);
        raw.push_all(nick);
        if user.is_some() {
            raw.push('!' as u8);
            raw.push_all(user.unwrap());
        }
        if host.is_some() {
            raw.push('@' as u8);
            raw.push_all(host.unwrap());
        }
        // instead of constructing a User directly, lets re-parse our raw string.
        // This way passing a nick of "foo!bar" or "foo@bar" will behave "properly".
        User::parse(raw)
    }

    pub fn raw<'a>(&'a self) -> &'a [u8] {
        self.raw.as_slice()
    }

    pub fn nick<'a>(&'a self) -> &'a [u8] {
        self.raw.slice_to(self.nicklen)
    }

    pub fn user<'a>(&'a self) -> Option<&'a [u8]> {
        self.user.map(|(a,b)| self.raw.slice(a, b))
    }

    pub fn host<'a>(&'a self) -> Option<&'a [u8]> {
        self.host.map(|(a,b)| self.raw.slice(a,b))
    }

    pub fn with_nick(&self, nick: &[u8]) -> User {
        User::new(nick, self.user(), self.host())
    }
}

impl Eq for User {
    fn eq(&self, other: &User) -> bool {
        self.raw == other.raw
    }
}

#[cfg(test)]
mod tests {
    use super::User;

    #[test]
    fn test_user_new() {
        let user = User::new(bytes!("nick"), Some(bytes!("user")), Some(bytes!("host")));
        assert_eq!(user.raw(), bytes!("nick!user@host"));
        assert_eq!(user.nick(), bytes!("nick"));
        assert_eq!(user.user(), Some(bytes!("user")));
        assert_eq!(user.host(), Some(bytes!("host")));

        let user = User::new(bytes!("nick"), Some(bytes!("user")), None);
        assert_eq!(user.raw(), bytes!("nick!user"));
        assert_eq!(user.nick(), bytes!("nick"));
        assert_eq!(user.user(), Some(bytes!("user")));
        assert_eq!(user.host(), None);

        let user = User::new(bytes!("nick"), None, Some(bytes!("host")));
        assert_eq!(user.raw(), bytes!("nick@host"));
        assert_eq!(user.nick(), bytes!("nick"));
        assert_eq!(user.user(), None);
        assert_eq!(user.host(), Some(bytes!("host")));

        let user = User::new(bytes!("nick"), None, None);
        assert_eq!(user.raw(), bytes!("nick"));
        assert_eq!(user.nick(), bytes!("nick"));
        assert_eq!(user.user(), None);
        assert_eq!(user.host(), None);
    }

    #[test]
    fn test_user_parse() {
        let user = User::parse(bytes!("bob!fred@joe.com"));
        assert_eq!(user.nick(), bytes!("bob"));
        assert_eq!(user.user(), Some(bytes!("fred")));
        assert_eq!(user.host(), Some(bytes!("joe.com")));

        let user = User::parse(bytes!("frob@whatever"));
        assert_eq!(user.nick(), bytes!("frob"));
        assert_eq!(user.user(), None);
        assert_eq!(user.host(), Some(bytes!("whatever")));

        let user = User::parse(bytes!("foo!baz"));
        assert_eq!(user.nick(), bytes!("foo"));
        assert_eq!(user.user(), Some(bytes!("baz")));
        assert_eq!(user.host(), None);

        let user = User::parse(bytes!("frobnitz"));
        assert_eq!(user.nick(), bytes!("frobnitz"));
        assert_eq!(user.user(), None);
        assert_eq!(user.host(), None);

        let user = User::parse(bytes!("host.ircserver.com"));
        assert_eq!(user.nick(), bytes!("host.ircserver.com"));
        assert_eq!(user.user(), None);
        assert_eq!(user.host(), None);
    }
}
