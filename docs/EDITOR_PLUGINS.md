# Editor plugins (in-process)

RedOx Studio loads **in-process Rust plugins** through `redox_editor_plugins::PluginHost`. Gameplay WASM scripts (`redox_script`) are a separate stack and are not editor plugins.

## Project manifest

List plugin names in `project.ron`:

```ron
(
    // ...
    plugins: ["hello"],
)
```

Built-in plugins today: `hello` (demo command `hello.greet` and a **Hello Plugin** dock tab).

## Plugin API v1

- `EditorPlugin`: `name()`, `api_version()` (must match `PLUGIN_API_VERSION`), `register(&mut PluginRegistry)`.
- `PluginRegistry`: `register_command`, `register_panel(id, title)`, `register_importer`, `register_validation_rule`.
- Incompatible `api_version` → plugin not enabled; warning in the log.

## Safe mode

When **Safe mode** is on in **Project → Plugins**, a panic in `register` or in a plugin command disables that plugin, logs an error, and shows a red banner. Other plugins and the editor keep running.

## Native cdylib plugins (📌 stub)

Feature `editor-native-plugins` on `redox_editor_plugins` documents the entry symbol `redox_editor_plugin_entry` but does **not** call `dlopen` yet. The UI shows a warning if you probe the loader. Prefer built-in plugins until the C ABI is stabilized.

## UI

**Project → Plugins…** lists built-ins, toggles the manifest list, and edits safe mode. Changes are written to `project.ron` on Save.