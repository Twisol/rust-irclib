//! Management of IRC server connection

use io_error = std::io::io_error::cond;
use std::io::{TcpStream,IpAddr};
use std::io::net::addrinfo;
use std::io::net::ip::SocketAddr;
use std::io::buffered::BufferedStream;
use std::{char,str,vec,uint};
use std::vec::MutableCloneableVector;
use std::cmp::min;

mod handlers;

/// Conn represenets a connection to a single IRC server
pub struct Conn<'a> {
    priv host: OptionsHost<'a>,
    priv tcp: BufferedStream<TcpStream>,
    priv logged_in: bool,
    priv nick: ~[u8],
    priv user: &'a str
}

/// OptionsHost allows for using an IP address or a host string
pub enum OptionsHost<'a> {
    Host(&'a str),
    Addr(IpAddr)
}

/// Options used with Conn for connecting to the server.
pub struct Options<'a> {
    host: OptionsHost<'a>,
    port: u16,
    nick: &'a str,
    user: &'a str
}

impl<'a> Options<'a> {
    /// Returns a new Options struct with default values
    pub fn new(host: &'a str, port: u16) -> Options<'a> {
        #[inline];
        Options {
            host: Host(host),
            port: port,
            nick: "ircnick",
            user: "ircuser"
        }
    }
}

/// Events that can be handled in the callback
pub enum Event {
    /// The connection was established
    Connected,
    /// A line was received from the server.
    /// This event is not sent until the user has successfully logged in.
    /// The first received line should be 001
    LineReceived(Line),
    /// The connection has terminated
    Disconnected
}

pub static DefaultPort: u16 = 6667;

/// Connects to the remote server. This method will not return until the connection
/// is terminated. Returns Ok(()) after connection termination if the connection was
/// established successfully, or Err(&str) if the connection could not be established in the
/// first place.
///
/// # Failure
///
/// Raises the `io_error` condition if an IO error happens at any point after the connection
/// is established.
pub fn connect(opts: Options, cb: |&mut Conn, Event|) -> Result<(),&'static str> {
    let addr = {
        match opts.host {
            Addr(x) => x,
            Host(host) => {
                let _guard = io_error.trap(|err| {
                    warn!("io_error resolving host address: {}", err.to_str());
                }).guard();
                match addrinfo::get_host_addresses(host) {
                    None | Some([]) => return Err("could not resolve host address"),
                    Some([x, ..]) => x
                }
            }
        }
    };
    let addr = SocketAddr{ ip: addr, port: opts.port };

    let stream = io_error.trap(|err| {
        warn!("io_error connecting to server: {}", err.to_str());
    }).inside(|| {
        // I don't know if ::connect() can throw io_error, but better safe than sorry
        TcpStream::connect(addr)
    });
    let stream = match stream {
        None => return Err("could not connect to server"),
        Some(tcp) => tcp
    };

    let mut conn = Conn{
        host: opts.host,
        tcp: BufferedStream::new(stream),
        logged_in: false,
        nick: opts.nick.as_bytes().to_owned(),
        user: opts.user
    };

    cb(&mut conn, Connected);

    conn.run(|c,e| cb(c,e));

    cb(&mut conn, Disconnected);

    Ok(())
}

impl<'a> Conn<'a> {
    fn run(&mut self, cb: |&mut Conn, Event|) {
        while !self.tcp.eof() {
            let mut line = match self.tcp.read_until('\n' as u8) {
                None => break,
                Some(line) => line
            };
            chomp(&mut line);
            let line = match Line::parse(line) {
                None => {
                    if cfg!(debug) {
                        let lines = str::from_utf8_opt(line);
                        if lines.is_some() {
                            debug!("Found non-parseable line: {}", lines.unwrap());
                        } else {
                            debug!("Found non-parseable line: {:?}", line);
                        }
                    }
                    continue;
                }
                Some(line) => line
            };
            handlers::handle_line(self, &line);
            if self.logged_in {
                cb(self, LineReceived(line));
            }
        }
    }

    /// Returns the host that was used to create this Conn
    pub fn host(&self) -> OptionsHost<'a> {
        self.host
    }

    /// Returns the user's nickname.
    /// If the user hasn't logged in yet, it returns the nickname that will be used.
    pub fn nick<'a>(&'a self) -> &'a [u8] {
        self.nick.as_slice()
    }

    /// Sends a command to the server.
    /// The line is truncated to 510 bytes (not including newline) before sending.
    ///
    /// If the command is an IRCCmd or IRCCode, the args vector is interpreted as a
    /// space-separated list of arguments, with a ':' argument prefix denoting the final
    /// (possibly space-containing) argument.
    ///
    /// If the command is an IRCAction, IRCCTCP, or IRCCTCPReply, the args vector is interpreted
    /// as the message that is being sent. It should be not be prefixed with a ':'.
    pub fn send_command(&mut self, cmd: Command, args: &[u8]) {
        let mut line = [0u8, ..512];
        let len = {
            let mut buf = line.mut_slice_to(510);

            fn append(buf: &mut &mut [u8], v: &[u8]) {
                let len = buf.copy_from(v);
                // this should work:
                //   *buf = buf.mut_slice_from(len);
                // but I'm getting weird borrowck issues (see mozilla/rust#11361)
                *buf = unsafe { ::std::cast::transmute(buf.mut_slice_from(len)) };
            }

            let is_ctcp = cmd.is_ctcp();
            match cmd {
                IRCCmd(cmd) => {
                    append(&mut buf, cmd.as_bytes());
                }
                IRCCode(code) => {
                    uint::to_str_bytes(code, 10, |v| {
                        append(&mut buf, v);
                    });
                }
                IRCAction(ref dst) | IRCCTCP(ref dst,_) => {
                    append(&mut buf, bytes!("PRIVMSG "));
                    append(&mut buf, *dst);
                    append(&mut buf, bytes!(" :\x01"));
                    let action = match cmd {
                        IRCAction(_) => bytes!("ACTION"),
                        IRCCTCP(_,ref action) => action.as_slice(),
                        _ => unreachable!()
                    };
                    append(&mut buf, action);
                }
                IRCCTCPReply(dst, action) => {
                    append(&mut buf, bytes!("NOTICE "));
                    append(&mut buf, dst);
                    append(&mut buf, bytes!(" :\x01"));
                    append(&mut buf, action);
                }
            }
            if !args.is_empty() {
                append(&mut buf, bytes!(" "));
                append(&mut buf, args);
            }
            if is_ctcp {
                append(&mut buf, bytes!("\x01"));
            }
            510 - buf.len()
        };
        line.mut_slice_from(len).copy_from(bytes!("\r\n"));
        self.tcp.write(line.slice_to(len+2));
    }

    /// Sets the user's nickname.
    pub fn set_nick<V: CopyableVector<u8>>(&mut self, nick: V) {
        let nick = nick.into_owned();
        self.send_command(IRCCmd(~"NICK"), nick);
        self.nick = nick;
    }

    /// Quits the connection
    pub fn quit(&mut self) {
        self.send_command(IRCCmd(~"QUIT"), []);
    }
}

fn chomp(s: &mut ~[u8]) {
    if s.ends_with(bytes!("\r\n")) {
        let len = s.len() - 2;
        s.truncate(len);
    } else if s.ends_with(bytes!("\n")) {
        let len = s.len() - 1;
        s.truncate(len);
    }
}

/// An IRC command
#[deriving(Eq,Clone)]
pub enum Command {
    /// An IRC command
    IRCCmd(~str),
    /// A 3-digit command code
    IRCCode(uint),
    /// CTCP actions. The first arg is the destination
    IRCAction(~[u8]),
    /// CTCP commands. The first arg is the command, the second is the destination
    IRCCTCP(~[u8], ~[u8]),
    /// CTCP replies. The first arg is the command, the second is the destination
    IRCCTCPReply(~[u8], ~[u8])
}

impl Command {
    /// Returns true if the command is a CTCP command
    pub fn is_ctcp(&self) -> bool {
        match *self {
            IRCAction(_) | IRCCTCP(_,_) | IRCCTCPReply(_,_) => true,
            _ => false
        }
    }
}

/// A parsed line
#[deriving(Eq,Clone)]
pub struct Line {
    /// The optional prefix
    prefix: Option<~[u8]>,
    /// The command
    command: Command,
    /// Any arguments
    args: ~[~[u8]],
}

impl Line {
    /// Parse a line into a Line struct
    pub fn parse(mut v: &[u8]) -> Option<Line> {
        let mut prefix = None;
        if v.starts_with(bytes!(":")) {
            let idx = match v.position_elem(&(' ' as u8)) {
                None => return None,
                Some(idx) => idx
            };
            prefix = Some(v.slice(1, idx).to_owned());
            v = v.slice_from(idx+1);
        }
        let (mut command, checkCTCP) = {
            let cmd;
            match v.position_elem(&(' ' as u8)) {
                Some(0) => return None,
                None => {
                    cmd = v;
                    v = &[];
                }
                Some(idx) => {
                    cmd = v.slice_to(idx);
                    v = v.slice_from(idx+1);
                }
            }
            if cmd.len() == 3 && cmd.iter().all(|&b| b >= '0' as u8 && b <= '9' as u8) {
                (IRCCode(uint::parse_bytes(cmd, 10).unwrap()), false)
            } else if cmd.iter().all(|&b| b < 0x80 && char::is_alphabetic(b as char)) {
                let shouldCheck = cmd == bytes!("PRIVMSG") || cmd == bytes!("NOTICE");
                (IRCCmd(str::from_utf8(cmd).to_owned()), shouldCheck)
            } else {
                return None;
            }
        };
        let mut args = ~[];
        while !v.is_empty() {
            if v[0] == ':' as u8 {
                args.push(v.slice_from(1).to_owned());
                break;
            }
            let idx = match v.position_elem(&(' ' as u8)) {
                None => {
                    args.push(v.to_owned());
                    break;
                }
                Some(idx) => idx
            };
            args.push(v.slice_to(idx).to_owned());
            v = v.slice_from(idx+1);
        }
        if checkCTCP && args.len() > 1 && args.last().starts_with([0x1]) {
            let mut text = args.pop();
            if text.len() > 1 && text.ends_with([0x1]) {
                text = text.slice(1,text.len()-1).to_owned();
            } else {
                text.shift();
            }
            let dst = args[0];
            let ctcpcmd;
            match text.position_elem(&(' ' as u8)) {
                Some(idx) => {
                    ctcpcmd = text.slice_to(idx).to_owned();
                    args = ~[text.slice_from(idx+1).to_owned()];
                }
                None => {
                    ctcpcmd = text.to_owned();
                    args = ~[];
                }
            }
            match command {
                IRCCmd(~"PRIVMSG") => {
                    if bytes!("ACTION") == ctcpcmd {
                        command = IRCAction(dst);
                    } else {
                        command = IRCCTCP(ctcpcmd, dst);
                    }
                }
                IRCCmd(~"NOTICE") => {
                    command = IRCCTCPReply(ctcpcmd, dst);
                }
                _ => unreachable!()
            }
        }
        Some(Line{
            prefix: prefix,
            command: command,
            args: args
        })
    }

    /// Converts into the "raw" representation :prefix cmd args
    pub fn to_raw(&self) -> ~[u8] {
        let mut cap = self.prefix.as_ref().map_default(0, |s| 1+s.len()+1);
        let mut found_space = false;
        cap += match self.command {
            IRCCmd(ref cmd) => cmd.len(),
            IRCCode(_) => 3,
            IRCAction(ref dst) => {
                "PRIVMSG".len() + 1 + dst.len() + 1 + ":\x01ACTION".len()
            }
            IRCCTCP(ref cmd, ref dst) => {
                "PRIVMSG".len() + 1 + dst.len() + 1 + 2 + cmd.len()
            }
            IRCCTCPReply(ref cmd, ref dst) => {
                "NOTICE".len() + 1 + dst.len() + 1 + 2 + cmd.len()
            }
        };
        if self.command.is_ctcp() {
            for arg in self.args.iter() {
                cap += 1 + arg.len();
            }
            cap += 1; // for the final \x01
        } else if !self.args.is_empty() {
            if self.args.len() > 1 {
                for arg in self.args.init().iter() {
                    cap += 1 + arg.len();
                }
            }
            let last = self.args.last();
            found_space = last.contains(&(' ' as u8));
            if found_space {
                cap += 1 + 1 /* : */ + last.len();
            } else {
                cap += 1 + last.len();
            }
        }
        let mut res = vec::with_capacity(cap);
        if self.prefix.is_some() {
            res.push(':' as u8);
            res.push_all(*self.prefix.as_ref().unwrap());
            res.push(' ' as u8);
        }
        match self.command {
            IRCCmd(ref cmd) => res.push_all(cmd.as_bytes()),
            IRCCode(c) => {
                uint::to_str_bytes(c, 10, |v| {
                    for _ in range(0, 3 - min(v.len(), 3)) {
                        res.push('0' as u8);
                    }
                    res.push_all(v);
                })
            }
            IRCAction(ref dst) => {
                res.push_all(bytes!("PRIVMSG "));
                res.push_all(*dst);
                res.push_all(bytes!(" :\x01ACTION"));
            }
            IRCCTCP(ref cmd, ref dst) => {
                res.push_all(bytes!("PRIVMSG "));
                res.push_all(*dst);
                res.push_all(bytes!(" :\x01"));
                res.push_all(cmd.as_slice());
            }
            IRCCTCPReply(ref cmd, ref dst) => {
                res.push_all(bytes!("NOTICE "));
                res.push_all(*dst);
                res.push_all(bytes!(" :\x01"));
                res.push_all(cmd.as_slice());
            }
        }
        if self.command.is_ctcp() {
            for arg in self.args.iter() {
                res.push(' ' as u8);
                res.push_all(*arg);
            }
            res.push(0x1);
        } else if !self.args.is_empty() {
            if self.args.len() > 1 {
                for arg in self.args.init().iter() {
                    res.push(' ' as u8);
                    res.push_all(*arg);
                }
            }
            res.push(' ' as u8);
            if found_space {
                res.push(':' as u8);
            }
            res.push_all(*self.args.last());
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::{Line,IRCCmd,IRCCode,IRCAction,IRCCTCP,IRCCTCPReply};

    #[test]
    fn parse_line() {
        macro_rules! b(
            ($val:expr) => (bytes!($val).to_owned())
        )
        macro_rules! t(
            ($v:expr, Some($exp:expr)) => (
                t!($v, Some($exp), $v);
            );
            ($v:expr, Some($exp:expr), $res:expr) => ({
                let v = $v;
                let exp = $exp;
                let line = Line::parse(v);
                assert!(line.is_some());
                let line = line.unwrap();
                assert_eq!(line.prefix, exp.prefix);
                assert_eq!(line.command, exp.command);
                assert_eq!(line.args, exp.args);
                let line = line.to_raw();
                assert_eq!(line, $res);
            });
            ($s:expr, None) => (
                assert_eq!(Line::parse($s), None);
            )
        )
        t!(b!(":sendak.freenode.net 001 asldfkj :Welcome to the freenode Internet \
            Relay Chat Network asldfkj"),
            Some(Line{
                prefix: Some(b!("sendak.freenode.net")),
                command: IRCCode(1),
                args: ~[b!("asldfkj"),
                        b!("Welcome to the freenode Internet Relay Chat Network asldfkj")]
            }));
        t!(b!("004 asdf :This is a test"),
            Some(Line{
                prefix: None,
                command: IRCCode(4),
                args: ~[b!("asdf"), b!("This is a test")]
            }));
        t!(b!(":nick!user@host.com PRIVMSG #channel :Some message"),
            Some(Line{
                prefix: Some(b!("nick!user@host.com")),
                command: IRCCmd(~"PRIVMSG"),
                args: ~[b!("#channel"), b!("Some message")]
            }));
        t!(b!(" :sendak.freenode.net 001 asdf :Test"), None);
        t!(b!(":sendak  001 asdf :Test"), None);
        t!(b!("004"),
            Some(Line{
                prefix: None,
                command: IRCCode(4),
                args: ~[]
            }));
        t!(b!(":bob!user@host.com PRIVMSG #channel :\x01ACTION does some stuff"),
            Some(Line{
                prefix: Some(b!("bob!user@host.com")),
                command: IRCAction(b!("#channel")),
                args: ~[b!("does some stuff")]
            }),
            b!(":bob!user@host.com PRIVMSG #channel :\x01ACTION does some stuff\x01"));
        t!(b!(":bob!user@host.com PRIVMSG #channel :\x01VERSION\x01"),
            Some(Line{
                prefix: Some(b!("bob!user@host.com")),
                command: IRCCTCP(b!("VERSION"), b!("#channel")),
                args: ~[]
            }));
        t!(b!(":bob NOTICE #frobnitz :\x01RESPONSE to whatever\x01"),
            Some(Line{
                prefix: Some(b!("bob")),
                command: IRCCTCPReply(b!("RESPONSE"), b!("#frobnitz")),
                args: ~[b!("to whatever")]
            }));
        t!(b!(":bob föo"), None);
        t!(b!(":bob f23"), None);
    }
}
