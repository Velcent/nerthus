# Diagnostics

Nerthus Diagnostics helps you find and fix problems with Nerthus, gathers what is needed to report one, and benchmarks your hardware. Open it from Help › Diagnostics…, Tools › Run Diagnostics…, or the Run Diagnostics button in Options › General. If Nerthus no longer starts properly, start it from a terminal with `nerthus --diagnostics`, which opens Diagnostics at once.

Choose a task and click **Start**:

* **Create a diagnostics package** – the logs, settings, hardware report, installation file list and running processes, zipped up. The package viewer opens when it is ready, with an option to copy the archive to your desktop. See [Diagnostics Package](diagnostics-package.md).
* **Run Nerthus in Verbose Mode** – Nerthus closes and starts again with Verbose Logging, which writes far more detail to its log.
* **Run Nerthus in Safe Mode** – Nerthus closes and starts again with the default preferences and options. Saved settings are only replaced if you change those preferences or options during that session.
* **Run Nerthus with CPU only** – Nerthus closes and starts again computing on the CPU alone. Recommended for crashes. See [CPU Only Mode](cpu-only-mode.md).
* **Reset to factory defaults** – puts all settings back to their defaults (copied aside first when **Backup current settings** is ticked), and shows the first-run setup again.
* **Cleanup autosaves, caches, and logs** – empties the data folder's `Logs` (but for the log being written), `Cache` and `Autosaves`.
* **Run Benchmark** – scores every compute device using 10 timed passes. **High Accuracy** uses 20 passes, and **Very High Accuracy** uses 50. See [Benchmark Devices](benchmark-devices.md).

When an update check has found a newer version, Diagnostics shows a warning and a link to the available releases.

When Nerthus crashes and you choose not to continue, it starts again with Diagnostics open.
