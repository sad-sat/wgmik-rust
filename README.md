# wgmik-server (Rust Edition)

A high-performance, ultra-low-resource bare-metal reimplementation of **wgmik-server** in Rust. Designed specifically for low-spec VPS instances, embedded systems, and resource-constrained servers without requiring Docker, Python, or external runtime dependencies.

---

## 🚀 Key Highlights

- **Single Standalone Executable**: Includes the backend API server, embedded SQLite database engine, in-process SVG chart rasterizer, and embedded React Web UI (~13 MB total binary size).
- **Minimal RAM Footprint**: Typically runs in **< 15–25 MB RAM** (vs 150–300+ MB for Python/Docker setups).
- **Zero External Runtime Dependencies**: No Docker, no Python, no Node.js, no headless browser required.
- **Fast Startup & Throughput**: Sub-millisecond route handling powered by Axum, Tokio, and Rustls.

---

## ⚡ Features & Parity

1. **MikroTik RouterOS Integration**:
   - Native async REST and binary API socket clients.
   - WireGuard interface & peer discovery, creation, modification, and deletion.
   - RouterOS 7.15+ compatibility validation.
   - Automatic Simple Queue provisioning for bandwidth throttling.

2. **Accurate Accounting Engine**:
   - Periodic 30-second polling loop.
   - 32-bit counter roll-over quarantine and reboot detection.
   - Minute-level, daily, and monthly rollup bucketing.

3. **Multi-Tiered Fair Usage Policy (FUP)**:
   - Flexible rule chains (global, per-router, or per-peer).
   - Configurable quotas (download-only, upload-only, or combined).
   - Multi-tier progressive throttle ladders (e.g. 50GB @ 10Mbps -> 100GB @ 2Mbps).
   - Automated RouterOS Simple Queue sync and cycle auto-reset.

4. **Bilingual & Dual-Calendar Support**:
   - Full Gregorian and Persian / Jalali calendar date math and month cycle resets.
   - English and Persian translations.

5. **Integrated Telegram Bot**:
   - User deep-link token onboarding.
   - Interactive commands (`/start`, `/today`, `/monthly`, `/alltime`, `/fair`, `/settings`, `/admin`).
   - In-process SVG/PNG chart generation rendered with Vazirmatn typography.

6. **Security & Management**:
   - JWT session management with session-version invalidation.
   - Passwords hashed with Bcrypt.
   - Router secrets and Telegram tokens encrypted via Python-compatible Fernet secret box.
   - Exclusive operation gate for maintenance locking.

---

## 🛠️ Quick Start

### 1. Run the Precompiled Binary
```bash
./target/release/wgmik-rust
```
By default, the server listens on `http://0.0.0.0:6574` and creates `wgmik.db` in the current working directory.

Open your browser at [http://localhost:6574](http://localhost:6574) to access the First-Run Setup Wizard and configure your initial admin account.

---

## ⚙️ Configuration (Environment Variables)

| Variable | Default | Description |
| :--- | :--- | :--- |
| `HOST` | `0.0.0.0` | IP address to bind |
| `PORT` | `6574` | Port to bind |
| `DATABASE_URL` | `sqlite:///./wgmik.db` | SQLite database URI |
| `SECRET_KEY` | *(Auto-generated & saved to `secret_key`)* | 32+ byte encryption key |
| `TIMEZONE` | `UTC` | Default timezone (e.g. `Asia/Tehran`, `UTC`) |
| `DATE_CALENDAR` | `gregorian` | Calendar system (`gregorian` or `persian`) |
| `MONTHLY_RESET_DAY` | `1` | Day of month for quota reset cycles (1–31) |
| `POLL_INTERVAL_SECONDS` | `30` | Interval for polling RouterOS peers |
| `ONLINE_THRESHOLD_SECONDS` | `15` | Handshake threshold for online status |

---

## 📦 Systemd Service (Bare-Metal Linux Deployment)

Create `/etc/systemd/system/wgmik.service`:

```ini
[Unit]
Description=wgmik-server WireGuard Accounting & Fair Usage Manager
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=/opt/wgmik
ExecStart=/opt/wgmik/wgmik-rust
Restart=always
RestartSec=5
Environment=HOST=0.0.0.0
Environment=PORT=6574
Environment=DATABASE_URL=sqlite:////opt/wgmik/wgmik.db

[Install]
WantedBy=multi-user.target
```

Enable and start:
```bash
sudo systemctl daemon-reload
sudo systemctl enable --now wgmik
```

---

## 🏗️ Building from Source

```bash
# 1. Build frontend (if modified)
cd frontend && npm run build && cd ..

# 2. Build optimized release binary
cargo build --release

# Binary is available at target/release/wgmik-rust
```
