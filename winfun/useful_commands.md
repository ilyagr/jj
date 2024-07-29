cargo nextest run --config-file winfun/nextest-config.toml -- test_git_push_to
export CARGO_TARGET_DIR=h:/tmp/rust-target
tailscale serve --tcp 2222 2222


# For sshfs + WinFSp
\\sshfs.r\100.108.54.52!2222\Users/ilyagr/share-winvm/
make sure there are no sshfs processes

... but don't use SSHFS. Mountain Duck is better.


