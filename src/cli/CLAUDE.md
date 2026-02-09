# CLI Command Organization Pattern

This document describes the organizational pattern for CLI commands in this application.

## Overview

All CLI commands follow a consistent modular structure that separates command definitions from their implementation logic. This pattern promotes:
- Clear separation of concerns
- Easy testability
- Consistent code organization
- Simple addition of new commands

## Pattern Structure

### 1. Command Enum (`src/cli/mod.rs`)

Each top-level CLI command must be represented as a variant in the `CliCommand` enum:

```rust
#[derive(Debug, Subcommand, Clone)]
pub enum CliCommand {
    /// Print the CLI version and exit
    Version(version::Version),
    /// Run the web server
    Server(server::Server),
    /// Export OpenAPI specification to stdout
    ExportOpenapi(export_openapi::ExportOpenapi),
}
```

**Requirements:**
- Each variant MUST have exactly one field
- The field must be a struct or enum with the same name as the variant (e.g., `Version(version::Version)`)
- The type can be a unit struct if the command has no arguments
- The type must implement clap's `Args` trait

### 2. Command Module Directory

Each command must have its own directory under `src/cli/`:
- `src/cli/version/` for the `Version` command
- `src/cli/server/` for the `Server` command
- `src/cli/export_openapi/` for the `ExportOpenapi` command

### 3. Command Module (`src/cli/<command>/mod.rs`)

Each command directory must contain a `mod.rs` file that:
- Defines the command struct
- Derives `Debug`, `Clone`, and clap's `Args`
- Implements a `dispatch` method that contains the business logic

**Example (unit struct with no arguments):**
```rust
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct Version;

impl Version {
    pub fn dispatch(self) -> miette::Result<()> {
        println!("{} v{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        Ok(())
    }
}
```

**Example (async dispatch with command-specific configuration):**
```rust
use crate::cli::{CorsConfig, PostgresConfig, TlsConfig};
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct Server {
    /// Server hostname
    #[arg(long = "host", env = "HOST", default_value = "127.0.0.1")]
    pub host: String,

    /// Server port
    #[arg(long = "port", env = "PORT", default_value = "8080")]
    pub port: u16,

    /// PostgreSQL database configuration
    #[command(flatten)]
    pub pg: PostgresConfig,

    /// TLS configuration
    #[command(flatten)]
    pub tls: TlsConfig,

    /// CORS configuration
    #[command(flatten)]
    pub cors: CorsConfig,
}

impl Server {
    pub async fn dispatch(self) -> miette::Result<()> {
        // Implementation using self.host, self.port, self.pg, etc.
        Ok(())
    }
}
```

### 4. Central Dispatch (`src/cli/mod.rs`)

The `CliCommand` enum implements an async `dispatch` method that delegates to each command's implementation:

```rust
impl CliCommand {
    pub async fn dispatch(self) -> miette::Result<()> {
        match self {
            CliCommand::Version(version) => version.dispatch(),
            CliCommand::Server(server) => server.dispatch().await,
            CliCommand::ExportOpenapi(export_openapi) => export_openapi.dispatch(),
        }
    }
}
```

**Key Points:**
- The method is async to support commands that need async operations
- Each command is self-contained with its own configuration
- Commands that need async can await, synchronous commands return immediately

### 5. Main Entry Point (`src/bin/main.rs`)

The main function parses the CLI and calls the central dispatch:

```rust
let cli = Cli::parse();

match cli.cmd {
    None => empty_command(),
    Some(cmd) => cmd.dispatch().await,
}
```

### 6. Global vs Command-Specific Configuration

**Global Configuration** (in `Cli` struct):
- Options that apply to all commands (e.g., `log_level`, `log_format`, `enable_colors`)
- Must be marked with `global = true` in clap attributes
- Available to all commands regardless of which subcommand is executed

**Command-Specific Configuration** (in command structs):
- Options that only apply to a specific command (e.g., `host`, `port` for `Server`)
- Defined as fields in the command's struct
- Only required when that specific command is executed
- Can use `#[command(flatten)]` to include shared configuration types

**Example:**
```rust
// Global configuration - applies to all commands (if needed)
#[derive(Debug, Parser, Clone)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Option<CliCommand>,
}

// Command-specific configuration - only for the Server command
#[derive(Debug, Clone, Args)]
pub struct Server {
    #[arg(long = "host", env = "HOST", default_value = "127.0.0.1")]
    pub host: String,

    #[arg(long = "port", env = "PORT", default_value = "8080")]
    pub port: u16,
}
```

This separation ensures that:
- Commands like `version` don't require unnecessary configuration
- Configuration validation only happens for the commands that use it
- The CLI is more user-friendly (fewer required arguments for simple commands)

## Adding a New Command

To add a new CLI command:

1. **Create the command directory:**
   ```bash
   mkdir src/cli/my_command
   ```

2. **Create `src/cli/my_command/mod.rs`:**
   ```rust
   use clap::Args;

   #[derive(Debug, Clone, Args)]
   pub struct MyCommand {
       // Add command-specific arguments here if needed
   }

   impl MyCommand {
       pub fn dispatch(self) -> miette::Result<()> {
           // Implement your command logic here
           Ok(())
       }
   }
   ```

3. **Register the module in `src/cli/mod.rs`:**
   ```rust
   pub mod my_command;
   ```

4. **Add the variant to `CliCommand` enum:**
   ```rust
   #[derive(Debug, Subcommand, Clone)]
   pub enum CliCommand {
       // ... existing commands
       /// Description of my command
       MyCommand(my_command::MyCommand),
   }
   ```

5. **Add the dispatch case:**
   ```rust
   impl CliCommand {
       pub async fn dispatch(self) -> miette::Result<()> {
           match self {
               // ... existing commands
               CliCommand::MyCommand(my_command) => my_command.dispatch(),
           }
       }
   }
   ```

6. **If your command needs configuration, add it as fields:**
   ```rust
   use clap::Args;

   #[derive(Debug, Clone, Args)]
   pub struct MyCommand {
       /// Command-specific argument
       #[arg(long, env = "MY_ARG")]
       pub my_arg: String,

       /// Command-specific flag
       #[arg(long)]
       pub my_flag: bool,
   }

   impl MyCommand {
       pub fn dispatch(self) -> miette::Result<()> {
           // Use self.my_arg, self.my_flag, etc.
           Ok(())
       }
   }
   ```

## Design Principles

- **Single Responsibility**: Each command module is responsible for one command's logic
- **Consistency**: All commands follow the same organizational pattern
- **Testability**: Command logic is isolated in testable functions
- **Modularity**: Commands can be added, removed, or modified without affecting others
- **Type Safety**: clap's derive macros provide compile-time validation of command structure

## Dispatch Method Signatures

All dispatch methods follow the pattern `dispatch(self)` and return `miette::Result<()>`:

- **Synchronous command:**
  ```rust
  pub fn dispatch(self) -> miette::Result<()>
  ```

- **Asynchronous command:**
  ```rust
  pub async fn dispatch(self) -> miette::Result<()>
  ```

Commands are self-contained and access their configuration through `self` fields. There is no need to pass the `Cli` struct to dispatch methods because:
- Global configuration is accessed through clap's global args mechanism if needed
- Command-specific configuration is defined as fields in the command struct
