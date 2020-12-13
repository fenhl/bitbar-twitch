This is a [BitBar](https://getbitbar.com/) plugin that shows live [Twitch](https://twitch.tv/) streams of users you follow.

# Installation

1. [Install BitBar](https://getbitbar.com/).
    * If you have [Homebrew](https://brew.sh/), you can also install with `brew install --cask bitbar`.
2. [Install Rust](https://www.rust-lang.org/tools/install).
    * If you have Homebrew, you can also install with `brew install rust`.
3. Install the plugin:
    ```sh
    cargo install --git=https://github.com/fenhl/bitbar-twitch --branch=main
    ```
4. Create a symlink to `~/.cargo/bin/bitbar-twitch` into your BitBar plugin folder. Call it something like `bitbar-twitch.1m.o`, where `1m` is the rate at which the list of streams will be refreshed.
5. Refresh BitBar by opening a menu and pressing <kbd>⌘</kbd><kbd>R</kbd>.
6. Follow the instructions in the menu to log in with your Twitch account.
