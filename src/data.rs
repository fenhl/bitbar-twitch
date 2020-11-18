use {
    std::{
        collections::{
            BTreeMap,
            BTreeSet,
        },
        fs::File,
    },
    chrono::prelude::*,
    serde::{
        Deserialize,
        Serialize,
    },
    twitch_helix::{
        Client,
        model::{
            GameId,
            StreamId,
            User,
            UserId,
        },
    },
    crate::Error,
};

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct Data {
    pub(crate) access_token: Option<String>,
    #[serde(default)]
    pub(crate) defer_deltas: Vec<Vec<String>>,
    pub(crate) deferred: Option<DateTime<Utc>>,
    #[serde(default)]
    pub(crate) hidden_games: BTreeMap<UserId, BTreeSet<GameId>>,
    #[serde(default)]
    pub(crate) hidden_streams: BTreeSet<StreamId>,
    user_id: Option<UserId>,
}

impl Data {
    pub(crate) async fn get_user_id(&mut self, client: &Client) -> Result<UserId, Error> {
        if let Some(ref user_id) = self.user_id { return Ok(user_id.clone()); }
        let id = User::me(client).await?.id;
        assert!(self.user_id.replace(id.clone()).is_none());
        Ok(id)
    }

    pub(crate) fn load() -> Result<Data, Error> {
        let dirs = xdg_basedir::get_data_home().into_iter().chain(xdg_basedir::get_data_dirs());
        Ok(dirs.filter_map(|data_dir| File::open(data_dir.join("bitbar/plugin-cache/twitch.json")).ok())
            .next().map_or(Ok(Data::default()), serde_json::from_reader)?)
    }

    pub(crate) fn save(&mut self) -> Result<(), Error> {
        let dirs = xdg_basedir::get_data_home().into_iter().chain(xdg_basedir::get_data_dirs());
        for data_dir in dirs {
            let data_path = data_dir.join("bitbar/plugin-cache/twitch.json");
            if data_path.exists() {
                if let Some(()) = File::create(data_path).ok()
                    .and_then(|data_file| serde_json::to_writer_pretty(data_file, &self).ok())
                {
                    return Ok(());
                }
            }
        }
        let data_path = xdg_basedir::get_data_home()?.join("bitbar/plugin-cache/twitch.json");
        let data_file = File::create(data_path)?;
        serde_json::to_writer_pretty(data_file, &self)?;
        Ok(())
    }
}
