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
    itertools::Itertools as _,
    notify_rust::Notification,
    serde::Deserialize,
    crate::{
        data::Data,
        paginated::PaginatedList
    }
};

mod data;
mod paginated;

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
    Reqwest(reqwest::Error),
    StreamType,
    Timespec(timespec::Error),
    UnclonableRequestBuilder,
    UnknownSubcommand(String)
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

#[derive(Debug, Deserialize)]
struct Follow {
    to_id: String
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
struct User {
    id: String,
    login: String,
    display_name: String
}

#[derive(Debug, Deserialize)]
struct Game {
    id: String,
    name: String
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
struct Stream {
    game_id: String,
    id: String,
    started_at: DateTime<Utc>,
    title: String,
    #[serde(rename = "type")]
    stream_type: StreamType,
    user_id: String,
    viewer_count: usize
}

impl Stream {
    fn error_for_type(self) -> Result<Stream, Error> {
        match self.stream_type {
            StreamType::Live => Ok(self),
            StreamType::Error => Err(Error::StreamType)
        }
    }

    fn menu_item(self, current_exe: &Path, user: &User) -> MenuItem {
        let time_live = Utc::now() - self.started_at;
        let time_live = if time_live >= Duration::hours(1) {
            format!("{}h {}m", time_live.num_hours(), time_live.num_minutes() % 60)
        } else {
            format!("{}m", time_live.num_minutes())
        };
        let channel_url = format!("https://twitch.tv/{}", user.login);
        ContentItem::new(&user.display_name).sub(vec![
            MenuItem::new(self.title),
            ContentItem::new(format!("Watch (Live for {})", time_live))
                .command(("/usr/local/bin/iina", &channel_url))
                .alt(ContentItem::new("Watch in Browser").href(channel_url).expect("invalid stream URL"))
                .into(),
            ContentItem::new(format!("Chat ({} Viewers)", self.viewer_count)).href(format!("https://www.twitch.tv/popout/{}/chat", user.login)).expect("invalid chat URL").into(),
            MenuItem::Sep,
            ContentItem::new("Hide This Stream").command((current_exe.display(), "hide-stream", &self.id)).refresh().into(),
            ContentItem::new("Hide This Game").command((current_exe.display(), "hide-game", &user.id, self.game_id)).refresh().into()
        ]).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
enum StreamType {
    #[serde(rename = "live")]
    Live,
    #[serde(rename = "")]
    Error
}

fn bitbar() -> Result<Menu, Error> {
    let current_exe = env::current_exe()?;
    let mut data = Data::load()?;
    if data.deferred.map_or(false, |deferred| deferred >= Utc::now()) {
        return Ok(Menu::default());
    }
    let access_token = data.access_token.as_ref().ok_or(Error::MissingAccessToken)?;
    let client = reqwest::Client::new();
    let follows = PaginatedList::from(
        client.get("https://api.twitch.tv/helix/users/follows")
            .query(&[("from_id", data.user_id.as_ref().ok_or(Error::MissingUserId)?)])
            .header("client-id", CLIENT_ID)
            .header("Authorization", format!("Bearer {}", access_token))
    );
    let mut users = HashMap::<String, User>::default();
    let online_streams = follows
        .chunks(100)
        .into_iter()
        .flat_map(|chunk| match chunk.collect::<Result<Vec<_>, _>>() {
            Ok(chunk) => {
                match PaginatedList::<User>::from(
                    client.get("https://api.twitch.tv/helix/users")
                        .query(&chunk.iter().map(|Follow { to_id, .. }| ("id", to_id)).collect::<Vec<_>>())
                        .header("client-id", CLIENT_ID)
                        .header("Authorization", format!("Bearer {}", access_token))
                ).map(|user| user.map(|user| (user.id.clone(), user)))
                .collect::<Result<Vec<_>, _>>() {
                    Ok(user_chunk) => { users.extend(user_chunk); }
                    Err(e) => { return Box::new(iter::once(Err(e))) as Box<dyn Iterator<Item = _>>; }
                }
                Box::new(PaginatedList::from(
                    client.get("https://api.twitch.tv/helix/streams")
                        .query(&chunk.iter().map(|Follow { to_id, .. }| ("user_id", to_id)).collect::<Vec<_>>())
                        .header("client-id", CLIENT_ID)
                        .header("Authorization", format!("Bearer {}", access_token))
                ))
            }
            Err(e) => Box::new(iter::once(Err(e))) as Box<dyn Iterator<Item = Result<Stream, _>>>
        })
        .map(|stream| stream.and_then(Stream::error_for_type))
        .collect::<Result<Vec<_>, _>>()?;
    let game_ids = online_streams.iter().map(|stream| stream.game_id.clone()).collect::<HashSet<_>>();
    let games = game_ids.into_iter()
        .chunks(100)
        .into_iter()
        .flat_map(|chunk|
            Box::new(PaginatedList::<Game>::from(
                client.get("https://api.twitch.tv/helix/games")
                    .query(&chunk.map(|game_id| ("id", game_id)).collect::<Vec<_>>())
                    .header("client-id", CLIENT_ID)
                    .header("Authorization", format!("Bearer {}", access_token))
            ))
        )
        .map(|result| result.map(|game| (game.id.clone(), game)))
        .collect::<Result<HashMap<_, _>, _>>()?;
    data.hidden_streams = data.hidden_streams.intersection(&online_streams.iter().map(|stream| stream.id.clone()).collect()).cloned().collect();
    data.save()?;
    let online_streams = online_streams.into_iter()
        .filter(|stream|
            !data.hidden_streams.contains(&stream.id)
            && data.hidden_games.get(&stream.user_id).map_or(true, |user_hidden_games| !user_hidden_games.contains(&stream.game_id))
        )
        .collect::<Vec<_>>();
    let mut streams_by_game = HashMap::<_, HashSet<_>>::default();
    for stream in &online_streams {
        streams_by_game.entry(stream.game_id.clone()).or_default().insert(stream.clone());
    }
    if online_streams.is_empty() { return Ok(Menu::default()); }
    Ok(vec![
        Ok(ContentItem::new(online_streams.len()).template_image(&include_bytes!("../assets/glitch.png")[..])?.into()),
        Ok(MenuItem::Sep)
    ].into_iter().chain(
        streams_by_game.into_iter()
            .sorted_by_key(|(_, streams)| -(streams.iter().map(|stream| stream.viewer_count).sum::<usize>() as isize))
            .flat_map(|(game_id, streams)|
                vec![
                    Ok(MenuItem::Sep),
                    Ok(MenuItem::new(games.get(&game_id).map_or(&game_id, |game| &game.name))),
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
        Error::Reqwest(e) => {
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
    data.hidden_games.entry(args.next().ok_or(Error::MissingCliArg)?).or_default().insert(args.next().ok_or(Error::MissingCliArg)?);
    data.save()?;
    Ok(())
}

fn hide_stream(mut args: impl Iterator<Item = String>) -> Result<(), Error> {
    let mut data = Data::load()?;
    data.hidden_streams.insert(args.next().ok_or(Error::MissingCliArg)?);
    data.save()?;
    Ok(())
}

fn main() {
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
        match bitbar() {
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
