# FigCraft · ijuice-j fork

An unofficial fork of [FigCraft](https://github.com/storytold/figcraft) by the ArtCraft team, with
**Google Fonts in the font picker**. Not made or supported by the ArtCraft team.

<p align="center">
  <img src="docs/images/shot-fork-google-fonts.png" alt="The font menu searched for 'pac': installed fonts first, then a Google Fonts section with Pacifico highlighted and a Download hint; beside it, text drawn in Pacifico after the download" width="760">
</p>

## What's different

- **Google Fonts and Noto in the font picker.** About 1,900 free families listed after your
  installed fonts and searchable with them. Pick one and it's applied and downloaded.
- **Asks first.** The first download asks *Allow downloading fonts?*; nothing goes online until
  you allow it.
- **`font.library` command** lists the same fonts for scripts and agents.
- **Workspaces.** Organise files as Workspace › Projects › Files on the home screen, like Figma's
  teams and projects. A workspace is a real folder (each project a subfolder), so it works with
  Explorer, cloud sync and backups.
  - New files are saved straight into **Drafts**, and workspace files auto-save.
  - Projects to create, rename and delete; files to move, rename, star and delete (to the Recycle
    Bin); several workspaces to switch between.
  - The editor shows where a file lives (*Workspace / Project*) under its name; click it to go there.
  - `workspace.*` commands do the same for scripts and agents.

## Build

```sh
git clone https://github.com/ijuice-j/figcraft
git clone https://github.com/storytold/craft-fonts
cd figcraft
cargo build --release -p figcraft
```

The app is `target/release/figcraft.exe`. On Windows with MinGW, clone into a folder without
spaces, or the exe gets no icon.

Decline FigCraft's in-app update offer: it would replace this fork with the official app.

## Branches

- `custom` (default): this fork.
- `main`: an untouched copy of the official FigCraft.

## Getting official updates

```sh
git fetch upstream
git log --oneline custom..upstream/main   # see what's new
git merge upstream/main                   # or: git cherry-pick <commit>
```

## License

MIT OR Apache-2.0, like FigCraft ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE),
[NOTICE](NOTICE)). The ArtCraft name and logos are the ArtCraft Team's trademarks and aren't part
of this license.
