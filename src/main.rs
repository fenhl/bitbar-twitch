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
        io,
        pin::pin,
    },
    bitbar::{
        ContentItem,
        Menu,
        MenuItem,
        attr::Command,
    },
    chrono::{
        Duration,
        prelude::*,
    },
    futures::stream::{
        self,
        StreamExt as _,
        TryStreamExt as _,
    },
    itertools::Itertools as _,
    thiserror::Error,
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

#[cfg(target_arch = "x86_64")] const IINA_PATH: &str = "/usr/local/bin/iina";
#[cfg(target_arch = "aarch64")] const IINA_PATH: &str = "/opt/homebrew/bin/iina";

#[derive(Debug, Error)]
enum Error {
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] Json(#[from] serde_json::Error),
    #[error(transparent)] Timespec(#[from] timespec::Error),
    #[error(transparent)] Twitch(twitch_helix::Error),
    #[error(transparent)] UrlParse(#[from] url::ParseError),
    #[error(transparent)] Xdg(#[from] xdg::BaseDirectoriesError),
    #[error("attempted to create command menu item with {0} args")]
    CommandLength(usize),
    #[error("timespec must not be empty")]
    EmptyTimespec,
    #[error("invalid or expired access token")]
    InvalidAccessToken,
    #[error("no access token configured")]
    MissingAccessToken,
    #[error("subcommand needs more parameters")]
    MissingCliArg,
    #[error("a followed user's data was lost")]
    MissingUserInfo,
    #[error("found OsString with invalid UTF-8")]
    OsString(OsString),
    #[error("unsupported stream type")]
    StreamType,
}

impl From<OsString> for Error {
    fn from(s: OsString) -> Self {
        Self::OsString(s)
    }
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

impl From<Error> for Menu {
    fn from(e: Error) -> Menu {
        let mut menu = Vec::default();
        match e {
            Error::InvalidAccessToken | Error::MissingAccessToken => {
                menu.push(MenuItem::new(e));
                menu.push(ContentItem::new("Log In")
                    .href("https://id.twitch.tv/oauth2/authorize?client_id=pe6plnyoh4yy8swie5nt80n84ynyft&redirect_uri=https%3A%2F%2Fbitbar-twitch.fenhl.net%2Fauth&response_type=token&scope=user:read:follows").expect("failed to parse the OAuth URL")
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
    fn menu_item(&self, user: &User) -> Result<MenuItem, Error>;
}

impl StreamExt for Stream {
    fn error_for_type(self) -> Result<Stream, Error> {
        match self.stream_type {
            StreamType::Live => Ok(self),
            StreamType::Error => Err(Error::StreamType),
        }
    }

    fn menu_item(&self, user: &User) -> Result<MenuItem, Error> {
        let time_live = Utc::now() - self.started_at;
        let time_live = if time_live >= Duration::hours(1) {
            format!("{}h {}m", time_live.num_hours(), time_live.num_minutes() % 60)
        } else {
            format!("{}m", time_live.num_minutes())
        };
        let channel_url = format!("https://twitch.tv/{}", user.login);
        Ok(ContentItem::new(&user.display_name).sub(vec![
            MenuItem::new(&self.title),
            ContentItem::new(format!("Watch (Live for {})", time_live))
                .command((IINA_PATH, &channel_url)).never_unwrap()
                .alt(ContentItem::new("Watch in Browser").href(channel_url)?)
                .into(),
            ContentItem::new(format!("Chat ({} Viewers)", self.viewer_count)).href(format!("https://www.twitch.tv/popout/{}/chat", user.login))?.into(),
            MenuItem::Sep,
            ContentItem::new("Hide This Stream").command(hide_stream(&self.id)?).never_unwrap().refresh().into(),
            ContentItem::new("Hide This Game").command(hide_game(&user.id, &self.game_id)?).never_unwrap().refresh().into(),
        ]).into())
    }
}

#[bitbar::command(varargs)]
fn defer(args: Vec<String>) -> Result<(), Error> {
    let mut data = Data::load()?;
    data.deferred = Some(if args.is_empty() {
        return Err(Error::MissingCliArg)
    } else {
        timespec::next(args)?.ok_or(Error::EmptyTimespec)?
    });
    data.save()?;
    Ok(())
}

#[bitbar::command]
fn hide_game(user_id: UserId, game_id: GameId) -> Result<(), Error> {
    let mut data = Data::load()?;
    data.hidden_games.entry(user_id).or_default().insert(game_id);
    data.save()?;
    Ok(())
}

#[bitbar::command]
fn hide_stream(stream_id: StreamId) -> Result<(), Error> {
    let mut data = Data::load()?;
    data.hidden_streams.insert(stream_id);
    data.save()?;
    Ok(())
}

#[bitbar::main(
    error_template_image = "../assets/glitch.png",
    commands(defer, hide_game, hide_stream),
)]
async fn main() -> Result<Menu, Error> {
    let current_exe = env::current_exe()?;
    let mut data = Data::load()?;
    if data.deferred.map_or(false, |deferred| deferred >= Utc::now()) {
        return Ok(Menu::default())
    }
    let access_token = data.access_token.as_ref().ok_or(Error::MissingAccessToken)?;
    let client = Client::new(concat!("bitbar-twitch/", env!("CARGO_PKG_VERSION")), CLIENT_ID, Credentials::from_oauth_token(access_token))?;
    let mut follows = pin!(Follow::from(&client, data.get_user_id(&client).await?).chunks(100));
    let mut users = HashMap::<UserId, User>::default();
    let mut online_streams = Vec::default();
    while let Some(chunk) = follows.next().await {
        let chunk = chunk.into_iter().collect::<Result<Vec<_>, _>>()?;
        let user_chunk = User::list(&client, chunk.iter().map(|Follow { broadcaster_id, .. }| broadcaster_id.clone()).collect())
            .map_ok(|user| (user.id.clone(), user))
            .try_collect::<Vec<_>>().await?;
        users.extend(user_chunk);
        let mut streams = pin!(Stream::list(&client, None, Some(chunk.into_iter().map(|Follow { broadcaster_id, .. }| broadcaster_id).collect()), None));
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
    if online_streams.is_empty() { return Ok(Menu::default()) }
    let mut menu = Menu(vec![
        ContentItem::new(online_streams.len()).template_image(&include_bytes!("../assets/glitch.png")[..])?.into(),
        MenuItem::Sep,
    ]);
    for (game_id, streams) in streams_by_game.into_iter()
        .sorted_by_key(|(_, streams)| -(streams.iter().map(|stream| stream.viewer_count).sum::<u64>() as isize))
    {
        menu.push(MenuItem::Sep);
        menu.push(MenuItem::new(games.get(&game_id).map_or(&game_id.0, |game| &game.name)));
        for stream in streams.into_iter().sorted_by_key(|stream| -(stream.viewer_count as isize)) {
            let user = users.get(&stream.user_id).ok_or(Error::MissingUserInfo)?;
            menu.push(stream.menu_item(user)?);
        }
    }
    if !data.defer_deltas.is_empty() {
        menu.push(MenuItem::Sep);
        for delta in &data.defer_deltas {
            menu.push(ContentItem::new(format!("Defer Until {}", delta.join(" ")))
                .command(
                    Command::try_from(
                        vec![&format!("{}", current_exe.display()), &format!("defer")]
                            .into_iter()
                            .chain(delta)
                            .collect::<Vec<_>>()
                    ).map_err(|v| Error::CommandLength(v.len()))?
                )?
                .refresh());
        }
    }
    Ok(menu)
}
