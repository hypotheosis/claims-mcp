# Install

Installing claims-mcp takes three steps: get the binary, register the server with your MCP
client, and install the `claims-audit` skill that teaches the agent the workflow.

## 1. Install the server

Pick one.

### Download a release

Each [GitHub release](https://github.com/hypotheosis/claims-mcp/releases) has an archive per
platform, a `.sha256` checksum for each, and the audit report for that version (`audit.pdf`,
the summary, and `audit-full.pdf`, every record):

| Platform | Archive |
|---|---|
| Linux x86_64 (static) | `claims-mcp-<version>-x86_64-unknown-linux-musl.tar.gz` |
| Linux ARM64 (static) | `claims-mcp-<version>-aarch64-unknown-linux-musl.tar.gz` |
| Windows x86_64 | `claims-mcp-<version>-x86_64-pc-windows-msvc.zip` |
| macOS Intel | `claims-mcp-<version>-x86_64-apple-darwin.tar.gz` |
| macOS Apple Silicon | `claims-mcp-<version>-aarch64-apple-darwin.tar.gz` |

On Linux and macOS:

```bash
sha256sum -c claims-mcp-<version>-<target>.tar.gz.sha256   # macOS: shasum -a 256 -c
tar -xzf claims-mcp-<version>-<target>.tar.gz
mkdir -p ~/.local/bin
install -m 755 claims-mcp-<version>-<target>/claims-mcp ~/.local/bin/
```

On Windows (PowerShell), compare the hash with the one in the `.sha256` file, then extract and
put `claims-mcp.exe` in a directory on your `PATH`:

```powershell
$archive = "claims-mcp-<version>-x86_64-pc-windows-msvc"
(Get-FileHash "$archive.zip" -Algorithm SHA256).Hash   # must match "$archive.zip.sha256"
Expand-Archive "$archive.zip" -DestinationPath .
```

Each archive also contains the README, `LICENSE`, `NOTICE` and the `skills/` directory. The
macOS binaries are not signed or notarized; if Gatekeeper blocks one, clear the quarantine flag
with `xattr -d com.apple.quarantine ~/.local/bin/claims-mcp`.

### Install with cargo

With Rust 1.89 or newer:

```bash
cargo install --locked --git https://github.com/hypotheosis/claims-mcp
```

This puts `claims-mcp` in `~/.cargo/bin`.

### Build from a clone

```bash
cargo build --release   # binary at target/release/claims-mcp
```

## 2. Register the server with Claude Code

With `claims-mcp` on your `PATH`, register it from the root of the project whose catalog it
should keep:

```bash
claude mcp add --scope project claims -- claims-mcp --catalog claims.json
```

`--scope project` writes `.mcp.json`, so everyone working in the repository gets the server.
Use `--scope user` to make it available in all your projects, or leave the flag off to register
it for this project only. A relative `--catalog` resolves against the directory Claude Code is
started in; an agent working in another checkout, such as a git worktree, names its own ledger
with each tool's [`catalog` parameter](../reference/tools.md#choosing-the-ledger). If you
prefer to write the `.mcp.json` entry by hand (use an absolute `command` path if the binary is
not on `PATH`):

```json
{
  "mcpServers": {
    "claims": {
      "command": "claims-mcp",
      "args": ["--catalog", "claims.json"]
    }
  }
}
```

Add the catalog's lock and temporary files to `.gitignore`: for `claims.json`, these are
`.claims.json.lock` and `.claims.json.tmp`.

Other MCP clients work the same way: run `claims-mcp --catalog <path>` as a stdio server. See
the [CLI reference](../reference/cli.md) for options.

## 3. Install the skill

The [`claims-audit`](https://github.com/hypotheosis/claims-mcp/blob/main/skills/claims-audit/SKILL.md)
skill teaches Claude the audit workflow. Copy it from a clone or a release archive, either for
yourself (all projects):

```bash
mkdir -p ~/.claude/skills
cp -r skills/claims-audit ~/.claude/skills/
```

or into a project, committed so collaborators get it too:

```bash
mkdir -p .claude/skills
cp -r skills/claims-audit .claude/skills/
```

On Windows the user directory is `%USERPROFILE%\.claude\skills`. Restart Claude Code, then
continue with [your first audit](first-audit.md).
