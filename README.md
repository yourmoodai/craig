# Craig

<img src="docs/assets/craig.png" width="180" alt="Craig wearing a blue hat">

Craig replaces detected offensive language in your Claude Code prompts with `ya ya ya`, using a filter that runs on your computer.

## Installation

**The first release is not published yet.** The commands below will work once it is available. Developers can [build from source](CONTRIBUTING.md#development).

You need Claude Code installed and a Mac with Apple silicon, or Windows/Linux on an Intel or AMD 64-bit computer.

1. Open **Terminal** on macOS/Linux, or **PowerShell** on Windows.
2. Paste these commands, one at a time:

   ```sh
   claude plugin marketplace add https://github.com/yourmoodai/craig/releases/latest/download/marketplace.json
   claude plugin install craig@yourmood.ai
   ```

3. Open Claude Code, enable Craig if prompted, and type `/craig` to check that it is ready.

The first launch automatically downloads the filter’s model (about 431 MB). This may take a few minutes. Wait until Craig is ready before submitting a prompt. If the download fails, open `/craig` to retry. No additional development tools are needed.

[Get help](https://github.com/yourmoodai/craig/issues) · [Advanced usage](docs/usage.md)
