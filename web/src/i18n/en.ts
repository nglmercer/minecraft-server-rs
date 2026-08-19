/** English strings. This file is the source of truth for every key. */
export const en = {
  common: {
    cancel: "Cancel",
    save: "Save",
    saving: "Saving…",
    delete: "Delete",
    deleting: "Deleting…",
    create: "Create",
    creating: "Creating…",
    rename: "Rename",
    download: "Download",
    close: "Close",
    confirm: "Confirm",
    loading: "Loading…",
    none: "—",
    search: "Search",
    searching: "Searching…",
    install: "Install",
    installing: "Installing…",
    restore: "Restore",
    extract: "Extract",
    extracting: "Extracting…",
    upload: "Upload",
    uploading: "Uploading…",
    name: "Name",
    optional: "Optional",
  },

  nav: {
    title: "Minecraft Panel",
    accounts: "Accounts",
    signOut: "Sign out",
    admin: "admin",
    language: "Language",
  },

  login: {
    heading: "Minecraft Panel",
    subtitle: "Sign in to manage your servers.",
    username: "Username",
    password: "Password",
    signIn: "Sign in",
    signingIn: "Signing in…",
    firstRun: "First run? The initial password was printed in the panel's console.",
    failed: "Login failed. Check the username and password.",
  },

  dashboard: {
    title: "Servers",
    summary: "{count} configured · {online} online",
    newServer: "New server",
    hostCpu: "Host CPU",
    hostMemory: "Host memory",
    serversOnline: "Servers online",
    empty: "No servers yet.",
    emptyAdmin: "Create one to get started — Java and the server jar are downloaded for you.",
    emptyUser: "Ask an administrator to grant you access to one.",
    manage: "Manage",
    start: "Start",
    restart: "Restart",
    stop: "Stop",
    kill: "Kill",
    upFor: "up {duration}",
  },

  status: {
    offline: "Offline",
    preparing: "Preparing",
    starting: "Starting",
    online: "Online",
    stopping: "Stopping",
    crashed: "Crashed",
  },

  createServer: {
    title: "New server",
    name: "Name",
    namePlaceholder: "Survival",
    port: "Port",
    flavour: "Flavour",
    proxy: "proxy",
    version: "Version",
    javaVersion: "Java version",
    javaHint: "Downloaded automatically if missing.",
    java: "Java {version}",
    minRam: "Min RAM (MB)",
    maxRam: "Max RAM (MB)",
    eulaPrefix: "I accept the",
    eulaLink: "Minecraft EULA",
    eulaSuffix: ". The server will not start until this is accepted.",
    submit: "Create server",
  },

  server: {
    back: "Servers",
    meta: "{core} {version} · port {port} · pid {pid} · up {uptime}",
    resources: "{cpu}% CPU · {memory} MB",
    eulaWarning:
      "The Minecraft EULA has not been accepted for this server, so it will refuse to start. Accept it under Settings.",
    tabs: {
      console: "Console",
      files: "Files",
      plugins: "Plugins",
      backups: "Backups",
      settings: "Settings",
    },
  },

  console: {
    title: "Console",
    live: "live",
    reconnecting: "reconnecting…",
    empty: "No output yet. Start the server to see its log here.",
    placeholder: "Type a command and press Enter",
    disconnected: "Not connected",
    skipped: "— {count} lines skipped (client fell behind) —",
  },

  files: {
    newFolder: "New folder",
    newFolderTitle: "New folder",
    newFolderLabel: "Folder name",
    newFile: "New file",
    newFilePlaceholder: "new-file.txt",
    createFile: "Create file",
    empty: "This folder is empty. It fills in once the server has run once.",
    uploaded: "Uploaded {count} file(s).",
    extracted: "Extracted {count} entries from {name}.",
    deleteTitle: "Delete {name}?",
    deleteBody: "This cannot be undone.",
    renameTitle: "Rename {name}",
    renameLabel: "New name",
    size: "Size",
    modified: "Modified",
    editing: "Editing {path}",
    tooLarge: "This file is too large to edit in the browser. Download it instead.",
  },

  plugins: {
    searchPlaceholder: "Search Modrinth for {core} {version} {kind}…",
    kindMods: "mods",
    kindPlugins: "plugins",
    scoped: "Results are filtered to what actually loads on {core} {version}.",
    downloads: "{count} downloads",
    noResults: "No results. Try a different search.",
    installed: "Installed",
    nothingInstalled: "Nothing installed yet.",
    installedOk: "Installed {name} {version} to {path}. Restart the server to load it.",
    unsupported:
      "A vanilla server cannot load plugins or mods. Switch this server to Paper (for plugins) or Fabric (for mods) under Settings — the world carries over.",
    deleteTitle: "Delete {name}?",
  },

  backups: {
    notePlaceholder: "Optional note — e.g. before the 1.21.9 upgrade",
    take: "Take backup",
    taking: "Archiving…",
    explain:
      "Backups capture worlds, configuration and plugins. The server jar and the downloadable libraries/, cache/ and versions/ trees are skipped, since the panel can fetch them again.",
    explainOnline: "The world is flushed to disk first, so the server can stay online.",
    empty: "No backups yet. Take one before your next upgrade.",
    created: "Created {id} ({size}).",
    restored: "Restored {id}.",
    restoreTitle: "Restore {id}?",
    restoreBody:
      "Files in the backup overwrite the current ones. Anything created since the backup is left alone.",
    deleteTitle: "Delete backup {id}?",
    deleteBody: "This cannot be undone.",
    mustStop: "Stop the server before restoring",
    runningWarning:
      "Restoring is disabled while the server is running — unpacking a world under a live JVM corrupts it. Stop the server first.",
  },

  settings: {
    serverSection: "Server",
    recoverySection: "Crash recovery",
    extraFlags: "Extra JVM flags",
    extraFlagsHint: "Space separated, inserted before -jar.",
    eulaAccepted: "Minecraft EULA accepted",
    maxRetries: "Max restart attempts",
    retryDelay: "Retry delay (s)",
    stopTimeout: "Graceful stop timeout (s)",
    autoRestart: "Restart automatically after a crash",
    saved: "Saved. Changes apply the next time the server starts.",
    saveChanges: "Save changes",
    removeServer: "Remove server",
    removeTitle: "Remove {name}?",
    removeBody: "It is removed from the panel. Its files stay on disk.",
  },

  users: {
    title: "Accounts",
    count: "{count} accounts",
    newAccount: "New account",
    you: "(you)",
    adminAll: "Full access to every server and account.",
    grantedCount: "{count} server(s) granted",
    setPassword: "Set password",
    setPasswordTitle: "New password for {name}",
    setPasswordLabel: "Password",
    passwordHint: "At least 8 characters.",
    makeAdmin: "Make admin",
    revokeAdmin: "Revoke admin",
    serverAccess: "Server access",
    deleteTitle: "Delete the account {name}?",
    deleteBody: "They are signed out immediately and lose access to every server.",
    cannotDeleteSelf: "You cannot delete your own account",
    adminLabel: "Administrator — full access to every server and account",
    username: "Username",
    password: "Password",
    createAccount: "Create account",
    passwordChanged: "Password changed for {name}. Their sessions were signed out.",
    roleChanged: "{name} is now {role}.",
    roleAdmin: "an admin",
    roleUser: "a regular user",
  },

  errors: {
    generic: "Something went wrong.",
    loadServers: "Could not load servers.",
    loadServer: "Could not load the server.",
    loadAccounts: "Could not load accounts.",
    actionFailed: "That action failed.",
    sessionExpired: "Your session expired. Sign in again.",
  },
} as const;

/**
 * The shape of `en`, with every leaf widened to `string`.
 *
 * Keeping the keys exact is the point — a translation missing a key, or
 * carrying one that no longer exists, is a compile error. Keeping the *values*
 * as literals would instead demand that Spanish say "Cancel".
 */
type Translations<T> = {
  [K in keyof T]: T[K] extends string ? string : Translations<T[K]>;
};

export type Dictionary = Translations<typeof en>;
