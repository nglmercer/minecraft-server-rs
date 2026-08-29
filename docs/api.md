# API

Everything is under `/api`. Browser sessions use an `HttpOnly` session cookie and a separate CSRF cookie. API clients may continue to use `Authorization: Bearer <token>`. A browser first calls `POST /servers/{id}/ws/ticket`, then connects to the WebSocket with its short-lived one-use `?ticket=`.

## Endpoints

| Method              | Path                                   | Purpose                            |
| ------------------- | -------------------------------------- | ---------------------------------- |
| `POST`              | `/auth/login`                          | Create a cookie session             |
| `POST`              | `/auth/logout`                         | Revoke the current session          |
| `GET`               | `/auth/me`                             | The current account                |
| `POST`              | `/auth/password`                       | Change your password               |
| `GET`               | `/playit/status`                       | Inspect Playit service state       |
| `GET`               | `/playit/account`                      | Inspect Playit account state (admin) |
| `POST`              | `/playit/claim`                       | Start the browser-based claim flow (admin) |
| `GET` `POST`        | `/playit/tunnels`                     | List / create tunnels (admin)      |
| `DELETE`            | `/playit/tunnels/{id}`                | Delete a tunnel (admin)            |
| `GET` `POST`        | `/servers`                             | List / create                      |
| `GET` `PATCH` `DELETE` | `/servers/{id}`                     | Inspect / reconfigure / remove     |
| `GET` `POST` `DELETE` | `/servers/{id}/playit`              | Inspect / attach / detach its tunnel |
| `POST`              | `/servers/{id}/power`                  | `start`, `stop`, `restart`, `kill` |
| `POST`              | `/servers/{id}/command`                | Send a console command             |
| `POST`              | `/servers/{id}/reinstall`              | Re-resolve and download the artifact |
| `GET`               | `/servers/{id}/logs`                   | The retained console buffer        |
| `WS`                | `/servers/{id}/ws`                     | Live console, both directions      |
| `POST`              | `/servers/{id}/ws/ticket`              | Short-lived one-use console grant  |
| `GET` `PUT` `DELETE`| `/servers/{id}/files`                  | List / write / delete              |
| `GET`               | `/servers/{id}/files/read`             | Read a text file                   |
| `GET`               | `/servers/{id}/files/sizes`            | Measure the subdirectories of a path |
| `POST`              | `/servers/{id}/files/ticket`           | Short-lived grant for one download  |
| `GET`               | `/servers/{id}/files/download`         | Stream a file out, given a ticket   |
| `POST`              | `/servers/{id}/files/upload`           | Multipart upload into a directory  |
| `POST`              | `/servers/{id}/files/extract`          | Unpack a `.zip`/`.jar`/`.tar.gz`   |
| `POST`              | `/servers/{id}/files/rename`           | Rename or move                     |
| `POST`              | `/servers/{id}/files/mkdir`            | Create a directory                 |
| `GET` `POST`        | `/servers/{id}/backups`                | List / take a backup               |
| `DELETE`            | `/servers/{id}/backups/{backup}`       | Delete a backup                    |
| `POST`              | `/servers/{id}/backups/{backup}/restore` | Restore (server must be stopped) |
| `POST`              | `/servers/{id}/backups/{backup}/ticket` | Short-lived grant for one download |
| `GET`               | `/servers/{id}/backups/{backup}/download` | Stream the archive, given a ticket |
| `GET`               | `/servers/{id}/mods`                   | Installed plugins or mods          |
| `GET`               | `/servers/{id}/mods/search`            | Search Modrinth, scoped to this server |
| `POST`              | `/servers/{id}/mods/install`           | Install a Modrinth project         |
| `GET` `POST`        | `/users`                               | List / create accounts (admin)     |
| `PATCH` `DELETE`    | `/users/{username}`                    | Update / delete an account (admin) |
| `GET`               | `/catalog/providers`                   | Installable server flavours        |
| `GET`               | `/catalog/{provider}/versions`         | Versions for a flavour             |
| `GET`               | `/catalog/{provider}/{version}/builds` | Builds for a version               |
| `GET`               | `/catalog/javas`                       | Java installations on the host      |
| `GET`               | `/system`                              | Host CPU, memory, servers online   |

Deleting a server removes it from the panel and leaves its files on disk. That is deliberate: a world should not be destroyable by a misclick in a browser.

## Playit

See [Playit](playit.md) for the embedded vs external mode distinction. Playit endpoints above are admin-only.

## Downloads — ticket model

A download is a browser navigation, and a browser cannot attach an `Authorization` header to one. Rather than putting the session token in the query string — where it lands in browser history, proxy logs and the panel's own request log — the client asks for a *ticket* first. A ticket names one file or one backup, expires after a minute, and grants nothing else. Console tickets expire faster, are bound to the issuing session, and are consumed at the handshake. Long-lived session tokens are never accepted in query strings.

Relevant endpoints: `/servers/{id}/files/ticket`, `/servers/{id}/files/download`, `/servers/{id}/backups/{backup}/ticket`, `/servers/{id}/backups/{backup}/download`, `/servers/{id}/ws/ticket`.

## Accounts and access

Admins see and manage everything. A regular account only reaches the servers it has been granted, across every endpoint including the console socket and the file manager. Changing an account's password or permissions revokes its existing sessions, so a demotion takes effect immediately rather than at the next login.

Unassigned servers are consistently hidden as `404` for server-scoped accounts. See [Security](security.md#trust-levels) for the full trust model.
