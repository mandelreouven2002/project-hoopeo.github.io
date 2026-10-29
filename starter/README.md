# First DEP application

Synthetic local tutorial for DEP Engine 0.5.2. Requires Engine and CLI installed separately, plus Python 3.10+ or Node.js 22+. Engine binaries have their own evaluation license; this starter is Apache-2.0, with the original Project Hoopeo examples credited to Reuven Mandelovich.

1. From this folder, run `dep-engine --listen 127.0.0.1:5543 --data-dir ./data --admin-password local-dev-only --init ./schema.sql`. Keep this terminal running. Initialization runs only on a new database. Port 5543 is intentionally separate from the default background service.
2. In a second terminal, run the Python or Node.js example below. All credentials and patient data are synthetic demonstration values, for loopback use only.

## Python

```sh
python3 -m venv .venv
. .venv/bin/activate
pip install -r python/requirements.txt
export DEP_DSN='host=127.0.0.1 port=5543 user=demo_app password=demo-local dbname=operational sslmode=disable'
python python/app.py
python python/verify.py
```

Expected lookup: `('Dana Cohen', None)`; expected partial-match refusal: `ZD101`. The verification script also tests roles, analytics, disclosure and the terminal rule.

## Node.js

```sh
cd node
npm ci
cp .env.example .env
npm start
npm run verify
```

Expected lookup: a Dana Cohen row with national_id null. The partial match is refused with ZD101.

## Inspect as admin

```sh
DEP_PASSWORD=local-dev-only dep-cli -h 127.0.0.1 -p 5543 -U admin -d operational
```

Run `SHOW PRIVACY MATRIX;` and `SHOW AUDIT 20;`.

## Reopening the tutorial

Stop the server with Ctrl+C. Re-run the server command to open the same data. To try a different schema, use a new data directory such as ./data-v2; 0.5.2 does not support ALTER TABLE. For remote connections use TLS and your own credentials.

The website's automated checks exercise schema and rules against the DEP.js simulator. These scripts are provided for explicit integration verification against your installed DEP Engine, not as a claim that the website build ran a native server.

Known native difference: both verification scripts expect sorting a disclosed ID to fail with ZD104, as required by specification section 8.9. A project review reports that native 0.5.1 accepts it; native 0.5.2 has not been retested here. An affected Engine will fail this check. Keep the failure visible until native enforcement is fixed.
