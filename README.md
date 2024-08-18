This is a BitBar plugin (supporting both [SwiftBar](https://swiftbar.app/) and [xbar](https://xbarapp.com/)) that shows live [Twitch](https://twitch.tv/) streams of users you follow.

# Installation

1. Install [SwiftBar](https://swiftbar.app/) or [xbar](https://xbarapp.com/).
    * If you're unsure which to install, I recommend SwiftBar, as this plugin has been tested with it.
    * If you have [Homebrew](https://brew.sh/), you can also install with `brew install --cask swiftbar` or `brew install --cask xbar`.
2. [Install Rust](https://www.rust-lang.org/tools/install).
    * If you have Homebrew, you can also install with `brew install rust`.
3. Install the plugin:
    ```sh
    cargo install --git=https://github.com/fenhl/bitbar-twitch --branch=main
    ```
4. Create a symlink to `~/.cargo/bin/bitbar-twitch` in your SwiftBar/xbar plugin folder. Call it something like `bitbar-twitch.1m.o`, where `1m` is the rate at which the list of streams will be refreshed.
5. If you're using SwiftBar, the plugin should now appear in your menu bar. If it doesn't appear automatically, or if you're using xbar, refresh by opening a menu and pressing <kbd>⌘</kbd><kbd>R</kbd>.
6. Follow the instructions in the menu to log in with your Twitch account.

# Notes

* Clicking “Watch” tries to open [IINA](https://iina.io/) by default. (If you have Homebrew, you can install it with `brew install --cask iina`.) By holding <kbd>⌥</kbd>, you can open streams in your browser instead.

# Configuration

The configuration file lives in a [JSON](https://json.org/) file at <code>[$XDG_DATA_DIRS](https://specifications.freedesktop.org/basedir-spec/basedir-spec-latest.html)/bitbar/plugin-cache/twitch.json</code>. It may contain the following entries, all optional:

* `accessToken`: A Twitch API key for the plugin. If this is missing, the plugin will display instructions for generating it.
* `deferDeltas`: An array of [timespecs](https://github.com/fenhl/timespec#syntax) given as arrays of strings. For each timespec listed, the plugin will generate menu items to hide itself until the next datetime matching that timespec.

Additionally, the entries `deferred`, `hiddenGames`, `hiddenStreams`, `hiddenUsers`, and `userId` are managed automatically by the plugin.

# Credits

This plugin is based on [the original Twitch Follows plugin](https://github.com/matryer/xbar-plugins/blob/6b85300ad6a039920a0b77a35933d73c2b0ba4d5/Web/Twitch/livestreamer-now-playing.5m.js) by [SCdF](https://github.com/SCdF), which [uses a now-defunct Twitch API](https://github.com/matryer/xbar-plugins/issues/1308) and [is no longer maintained](https://github.com/matryer/xbar-plugins/issues/1308#issuecomment-535840433).
