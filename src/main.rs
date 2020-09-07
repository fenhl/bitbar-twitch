#![deny(rust_2018_idioms, unused, unused_import_braces, unused_qualifications, warnings)]

use {
    std::{
        collections::{
            HashMap,
            HashSet
        },
        convert::Infallible,
        env,
        fmt,
        io,
        iter,
        path::Path,
        process::Output
    },
    bitbar::{
        Command,
        ContentItem,
        Menu,
        MenuItem
    },
    chrono::{
        Duration,
        prelude::*
    },
    derive_more::From,
    futures::stream::{
        self,
        StreamExt as _,
        TryStreamExt as _
    },
    itertools::Itertools as _,
    notify_rust::Notification,
    pin_utils::pin_mut,
    twitch_helix::{
        Client,
        model::{
            Follow,
            Game,
            GameId,
            Stream,
            StreamId,
            StreamType,
            User,
            UserId
        }
    },
    crate::data::Data
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
    Io(io::Error),
    Json(serde_json::Error),
    MissingAccessToken,
    MissingCliArg,
    MissingUserId,
    MissingUserInfo,
    StreamType,
    Timespec(timespec::Error),
    Twitch(twitch_helix::Error)
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

trait ResultNeverExt<T> {
    fn never_unwrap(self) -> T;
}

impl<T> ResultNeverExt<T> for Result<T, Infallible> {
    fn never_unwrap(self) -> T {
        match self {
            Ok(inner) => inner,
            Err(never) => match never {}
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
            StreamType::Error => Err(Error::StreamType)
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
            ContentItem::new("Hide This Stream").command((current_exe.display(), "hide-stream", &self.id)).refresh().into(),
            ContentItem::new("Hide This Game").command((current_exe.display(), "hide-game", &user.id, &self.game_id)).refresh().into()
        ]).into()
    }
}

async fn bitbar() -> Result<Menu, Error> {
    let current_exe = env::current_exe()?;
    let mut data = Data::load()?;
    if data.deferred.map_or(false, |deferred| deferred >= Utc::now()) {
        return Ok(Menu::default());
    }
    let access_token = data.access_token.as_ref().ok_or(Error::MissingAccessToken)?;
    let client = Client::new(concat!("bitbar-twitch/", env!("CARGO_PKG_VERSION")), CLIENT_ID, access_token)?;
    let follows = Follow::from(&client, data.user_id.as_ref().ok_or(Error::MissingUserId)?.clone()).chunks(100);
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
        Ok(MenuItem::Sep)
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

fn notify(summary: impl fmt::Display, body: impl fmt::Display) {
    //let _ = notify_rust::set_application(&notify_rust::get_bundle_identifier_or_default("BitBar")); //TODO uncomment when https://github.com/h4llow3En/mac-notification-sys/issues/8 is fixed
    let _ = Notification::default()
        .summary(&summary.to_string())
        .sound_name("Funk")
        .body(&body.to_string())
        .show();
}

trait ResultExt {
    type Ok;

    fn notify(self, summary: impl fmt::Display) -> Self::Ok;
}

impl<T, E: fmt::Debug> ResultExt for Result<T, E> {
    type Ok = T;

    fn notify(self, summary: impl fmt::Display) -> T {
        match self {
            Ok(t) => t,
            Err(e) => {
                notify(&summary, format!("{:?}", e));
                panic!("{}: {:?}", summary, e);
            }
        }
    }
}

fn error_menu(e: Error, menu: &mut Vec<MenuItem>) {
    match e {
        Error::MissingAccessToken => {
            menu.push(MenuItem::new("no access token configured"));
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
        _ => { menu.push(MenuItem::new(format!("error: {:?}", e))); }
    }
}

// subcommands

fn defer(args: impl Iterator<Item = String>) -> Result<(), Error> {
    let mut args = args.peekable();
    let mut data = Data::load()?;
    data.deferred = Some(if args.peek().is_some() {
        timespec::next(args)?.ok_or(Error::EmptyTimespec)?
    } else {
        return Err(Error::MissingCliArg);
    });
    data.save()?;
    Ok(())
}

fn hide_game(mut args: impl Iterator<Item = String>) -> Result<(), Error> {
    let mut data = Data::load()?;
    data.hidden_games.entry(UserId(args.next().ok_or(Error::MissingCliArg)?)).or_default().insert(GameId(args.next().ok_or(Error::MissingCliArg)?));
    data.save()?;
    Ok(())
}

fn hide_stream(mut args: impl Iterator<Item = String>) -> Result<(), Error> {
    let mut data = Data::load()?;
    data.hidden_streams.insert(StreamId(args.next().ok_or(Error::MissingCliArg)?));
    data.save()?;
    Ok(())
}

#[tokio::main]
async fn main() {
    let mut args = env::args().skip(1);
    if let Some(arg) = args.next() {
        match &arg[..] {
            "defer" => defer(args).notify("error in defer cmd"),
            "hide-game" => hide_game(args).notify("error in hide-game cmd"),
            "hide-stream" => hide_stream(args).notify("error in hide-stream cmd"),
            _ => {
                notify("error in bitbar-twitch", format!("unknown subcommand: {}", arg));
                panic!("unknown subcommand: {}", arg);
            }
        }
    } else {
        match bitbar().await {
            Ok(menu) => { print!("{}", menu); }
            Err(e) => {
                let mut menu = vec![
                    ContentItem::new("?").template_image(&include_bytes!("../assets/glitch.png")[..]).never_unwrap().into(),
                    MenuItem::Sep
                ];
                error_menu(e, &mut menu);
                print!("{}", Menu(menu));
            }
        }
    }
}
