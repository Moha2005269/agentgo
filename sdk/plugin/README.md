# AgentGo plugins

AgentGo is extended with data, not native code:

- **Skills**: a folder with `skill.json` (steps with `{{params}}`), see
  `template/`. `skill.install_local {path}` installs it and `skill.test` runs it.
- **User scripts**: `script.user_install` (the person installs them). They run
  on matching pages in their own isolated world.
- **Extensions**: `extension.install` runs a Chrome extension's content scripts.
