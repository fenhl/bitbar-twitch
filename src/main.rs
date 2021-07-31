#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
#![forbid(unsafe_code)]

use {
    std::{
        collections::{
            HashMap,
            HashSet,
        },
        convert::Infallible,
        env,
        ffi::OsString,
        fmt,
        io,
        iter,
        path::Path,
        process::Output,
    },
    bitbar::{
        Command,
        ContentItem,
        Menu,
        MenuItem,
    },
    chrono::{
        Duration,
        prelude::*,
    },
    derive_more::From,
    futures::{
        pin_mut,
        stream::{
            self,
            StreamExt as _,
            TryStreamExt as _,
        },
    },
    itertools::Itertools as _,
    twitch_helix::{
        Client,
        Credentials,
        model::{
            Follow,
            Game,
            GameId,
            Stream,
            StreamId,
            StreamType,
            User,
            UserId,
        },
    },
    crate::data::Data,
};

mod data;

const CLIENT_ID: &str = "pe6plnyoh4yy8swie5nt80n84ynyft";

#[derive(Debug, From)]
enum Error {
    Basedir(xdg_basedir::Error),
    CommandExit(&'static str, Output),
    #[from(ignore)]
    CommandLength(usize),
    EmptyTimespec,
    InvalidAccessToken,
    Io(io::Error),
    Json(serde_json::Error),
    MissingAccessToken,
    MissingCliArg,
    MissingUserInfo,
    OsString(OsString),
    StreamType,
    Timespec(timespec::Error),
    #[from(ignore)]
    Twitch(twitch_helix::Error),
}

impl From<twitch_helix::Error> for Error {
    fn from(e: twitch_helix::Error) -> Error {
        if let twitch_helix::Error::HttpStatus(ref e, _ /*body*/) = e {
            if e.status() == Some(reqwest::StatusCode::UNAUTHORIZED) { //TODO also check body
                return Error::InvalidAccessToken
            }
        }
        Error::Twitch(e)
    }
}

impl From<Infallible> for Error {
    fn from(never: Infallible) -> Error {
        match never {}
    }
}

trait CommandOutputExt {
    type Ok;

    fn check(self, name: &'static str) -> Result<Self::Ok, Error>;
}

impl CommandOutputExt for Output {
    type Ok = Output;

    fn check(self, name: &'static str) -> Result<Output, Error> {
        if self.status.success() {
            Ok(self)
        } else {
            Err(Error::CommandExit(name, self))
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Basedir(e) => e.fmt(f),
            Error::CommandExit(cmd, output) => write!(f, "command `{}` exited with {}", cmd, output.status),
            Error::CommandLength(len) => write!(f, "attempted to create command menu item with {} args", len),
            Error::EmptyTimespec => write!(f, "timespec must not be empty"),
            Error::Io(e) => write!(f, "I/O error: {}", e),
            Error::Json(e) => write!(f, "JSON error: {}", e),
            Error::InvalidAccessToken => write!(f, "invalid or expired access token"),
            Error::MissingAccessToken => write!(f, "no access token configured"),
            Error::MissingCliArg => write!(f, "subcommand needs more parameters"),
            Error::MissingUserInfo => write!(f, "a followed user's data was lost"),
            Error::OsString(_) => write!(f, "found OsString with invalid UTF-8"),
            Error::StreamType => write!(f, "unsupported stream type"),
            Error::Timespec(e) => write!(f, "timespec error: {:?}", e), //TODO implement Display for timespec::Error and use here
            Error::Twitch(e) => e.fmt(f),
        }
    }
}

impl From<Error> for Menu {
    fn from(e: Error) -> Menu {
        let mut menu = Vec::default();
        match e {
            Error::InvalidAccessToken | Error::MissingAccessToken => {
                menu.push(MenuItem::new(e));
                menu.push(ContentItem::new("Log In")
                    .href("https://id.twitch.tv/oauth2/authorize?client_id=pe6plnyoh4yy8swie5nt80n84ynyft&redirect_uri=https%3A%2F%2Fgithub.com%2Ffenhl%2Fbitbar-twitch%2Fwiki%2Foauth-landing&response_type=token&scope=").expect("failed to parse the OAuth URL")
                    .color("blue").expect("failed to parse the color blue")
                    .into());
            }
            Error::Twitch(twitch_helix::Error::HttpStatus(e, body)) => {
                let url = e.url().expect("missing URL in HTTP status error");
                menu.push(ContentItem::new(&e)
                    .href(url.clone()).expect("failed to parse the request error URL")
                    .color("blue").expect("failed to parse the color blue")
                    .into());
                if let Ok(body) = body {
                    menu.push(MenuItem::new(body));
                }
            }
            Error::Twitch(twitch_helix::Error::Reqwest(e)) => {
                menu.push(MenuItem::new(format!("reqwest error: {}", e)));
                if let Some(url) = e.url() {
                    menu.push(ContentItem::new(format!("URL: {}", url))
                        .href(url.clone()).expect("failed to parse the request error URL")
                        .color("blue").expect("failed to parse the color blue")
                        .into());
                }
            }
            _ => {
                menu.push(MenuItem::new(&e));
                menu.push(MenuItem::new(format!("{:?}", e)));
            }
        }
        Menu(menu)
    }
}

trait ResultNeverExt<T> {
    fn never_unwrap(self) -> T;
}

impl<T> ResultNeverExt<T> for Result<T, Infallible> {
    fn never_unwrap(self) -> T {
        match self {
            Ok(inner) => inner,
            Err(never) => match never {},
        }
    }
}

trait StreamExt: Sized {
    fn error_for_type(self) -> Result<Self, Error>;
    fn menu_item(&self, current_exe: &Path, user: &User) -> MenuItem;
}

impl StreamExt for Stream {
    fn error_for_type(self) -> Result<Stream, Error> {
        match self.stream_type {
            StreamType::Live => Ok(self),
            StreamType::Error => Err(Error::StreamType),
        }
    }

    fn menu_item(&self, current_exe: &Path, user: &User) -> MenuItem {
        let time_live = Utc::now() - self.started_at;
        let time_live = if time_live >= Duration::hours(1) {
            format!("{}h {}m", time_live.num_hours(), time_live.num_minutes() % 60)
        } else {
            format!("{}m", time_live.num_minutes())
        };
        let channel_url = format!("https://twitch.tv/{}", user.login);
        ContentItem::new(&user.display_name).sub(vec![
            MenuItem::new(&self.title),
            ContentItem::new(format!("Watch (Live for {})", time_live))
                .command(("/usr/local/bin/iina", &channel_url))
                .alt(ContentItem::new("Watch in Browser").href(channel_url).expect("invalid stream URL"))
                .into(),
            ContentItem::new(format!("Chat ({} Viewers)", self.viewer_count)).href(format!("https://www.twitch.tv/popout/{}/chat", user.login)).expect("invalid chat URL").into(),
            MenuItem::Sep,
            ContentItem::new("Hide This Stream").command((current_exe.display(), "hide_stream", &self.id)).refresh().into(),
            ContentItem::new("Hide This Game").command((current_exe.display(), "hide_game", &user.id, &self.game_id)).refresh().into(),
        ]).into()
    }
}

#[bitbar::command]
fn defer(args: impl Iterator<Item = OsString>) -> Result<(), Error> {
    let mut args = args.peekable();
    let mut data = Data::load()?;
    data.deferred = Some(if args.peek().is_some() {
        timespec::next(args.map(OsString::into_string).collect::<Result<Vec<_>, _>>()?)?.ok_or(Error::EmptyTimespec)?
    } else {
        return Err(Error::MissingCliArg);
    });
    data.save()?;
    Ok(())
}

#[bitbar::command]
fn hide_game(mut args: impl Iterator<Item = OsString>) -> Result<(), Error> {
    let mut data = Data::load()?;
    data.hidden_games.entry(UserId(args.next().ok_or(Error::MissingCliArg)?.into_string()?)).or_default().insert(GameId(args.next().ok_or(Error::MissingCliArg)?.into_string()?));
    data.save()?;
    Ok(())
}

#[bitbar::command]
fn hide_stream(mut args: impl Iterator<Item = OsString>) -> Result<(), Error> {
    let mut data = Data::load()?;
    data.hidden_streams.insert(StreamId(args.next().ok_or(Error::MissingCliArg)?.into_string()?));
    data.save()?;
    Ok(())
}

#[bitbar::main(error_template_image = "../assets/glitch.png")]
async fn main() -> Result<Menu, Error> {
    let current_exe = env::current_exe()?;
    let mut data = Data::load()?;
    if data.deferred.map_or(false, |deferred| deferred >= Utc::now()) {
        return Ok(Menu::default());
    }
    let access_token = data.access_token.as_ref().ok_or(Error::MissingAccessToken)?;
    let client = Client::new(concat!("bitbar-twitch/", env!("CARGO_PKG_VERSION")), CLIENT_ID, Credentials::from_oauth_token(access_token))?;
    let follows = Follow::from(&client, data.get_user_id(&client).await?).chunks(100);
    pin_mut!(follows);
    let mut users = HashMap::<UserId, User>::default();
    let mut online_streams = Vec::default();
    while let Some(chunk) = follows.next().await {
        let chunk = chunk.into_iter().collect::<Result<Vec<_>, _>>()?;
        let user_chunk = User::list(&client, chunk.iter().map(|Follow { to_id, .. }| to_id.clone()).collect())
            .map_ok(|user| (user.id.clone(), user))
            .try_collect::<Vec<_>>().await?;
        users.extend(user_chunk);
        let streams = Stream::list(&client, None, Some(chunk.into_iter().map(|Follow { to_id, .. }| to_id).collect()), None);
        pin_mut!(streams);
        while let Some(stream) = streams.try_next().await? {
            online_streams.push(stream.error_for_type()?);
        }
    }
    let game_ids = online_streams.iter().map(|stream| stream.game_id.clone()).collect::<HashSet<_>>();
    let games = stream::iter(game_ids)
        .chunks(100)
        .map(|chunk| Ok::<_, Error>(
            Game::list(&client, chunk.into_iter().collect()).map_err(Error::from)
        ))
        .try_flatten()
        .map_ok(|game| (game.id.clone(), game))
        .try_collect::<HashMap<_, _>>().await?;
    data.hidden_streams = data.hidden_streams.intersection(&online_streams.iter().map(|stream| stream.id.clone()).collect()).cloned().collect();
    data.save()?;
    let online_streams = online_streams.into_iter()
        .filter(|stream|
            !data.hidden_streams.contains(&stream.id)
            && data.hidden_games.get(&stream.user_id).map_or(true, |user_hidden_games| !user_hidden_games.contains(&stream.game_id))
        )
        .collect::<Vec<_>>();
    let mut streams_by_game = HashMap::<_, Vec<_>>::default();
    for stream in &online_streams {
        streams_by_game.entry(stream.game_id.clone()).or_default().push(stream.clone());
    }
    if online_streams.is_empty() { return Ok(Menu::default()); }
    Ok(vec![
        Ok(ContentItem::new(online_streams.len()).template_image(&include_bytes!("../assets/glitch.png")[..])?.into()),
        Ok(MenuItem::Sep),
    ].into_iter().chain(
        streams_by_game.into_iter()
            .sorted_by_key(|(_, streams)| -(streams.iter().map(|stream| stream.viewer_count).sum::<u64>() as isize))
            .flat_map(|(game_id, streams)|
                vec![
                    Ok(MenuItem::Sep),
                    Ok(MenuItem::new(games.get(&game_id).map_or(&game_id.0, |game| &game.name))),
                ].into_iter()
                    .chain(streams.into_iter().sorted_by_key(|stream| -(stream.viewer_count as isize)).map(|stream| {
                        let user = users.get(&stream.user_id).ok_or(Error::MissingUserInfo)?;
                        Ok(stream.menu_item(&current_exe, user))
                    }))
            )
    ).chain(if data.defer_deltas.is_empty() {
        Vec::default()
    } else {
        iter::once(Ok(MenuItem::Sep)).chain(
            data.defer_deltas.iter().map(|delta| Ok(
                ContentItem::new(format!("Defer Until {}", delta.join(" ")))
                    .command(
                        Command::try_from(
                            vec![&format!("{}", current_exe.display()), &format!("defer")]
                                .into_iter()
                                .chain(delta)
                                .collect::<Vec<_>>()
                        ).map_err(|v| Error::CommandLength(v.len()))?
                    )
                    .refresh()
                    .into()
            ))
        )
        .collect()
    }).collect::<Result<Menu, Error>>()?)
}
