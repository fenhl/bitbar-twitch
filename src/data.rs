use {
    std::{
        collections::{
            BTreeMap,
            BTreeSet,
        },
        fs,
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
    xdg::BaseDirectories,
    crate::Error,
};

const PATH: &str = "bitbar/plugin-cache/twitch.json";

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
    pub(crate) async fn get_user_id(&mut self, client: &Client<'_>) -> Result<UserId, Error> {
        if let Some(ref user_id) = self.user_id { return Ok(user_id.clone()); }
        let id = User::me(client).await?.id;
        assert!(self.user_id.replace(id.clone()).is_none());
        Ok(id)
    }

    pub(crate) fn load() -> Result<Data, Error> {
        Ok(if let Some(path) = BaseDirectories::new()?.find_data_file(PATH) {
            serde_json::from_slice(&fs::read(path)?)?
        } else {
            Data::default()
        })
    }

    pub(crate) fn save(&self) -> Result<(), Error> {
        let path = BaseDirectories::new()?.place_data_file(PATH)?;
        let mut buf = serde_json::to_string_pretty(self)?;
        buf.push('\n');
        fs::write(path, buf)?;
        Ok(())
    }
}
