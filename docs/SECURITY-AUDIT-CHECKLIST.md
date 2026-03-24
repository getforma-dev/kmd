# Security Audit Checklist

> **Purpose:** Reusable, machine-readable security reference for building and auditing web applications. Use this as a checklist when finishing an app -- go down the list, verify every item, and check it off.
>
> **How to use:** Each item has a checkbox `- [ ]`, a severity rating, and (where applicable) a CWE/OWASP reference. Search for specific attack types with grep/ctrl-F. Items are ordered by category, then by severity within each category.
>
> **Severity key:** `[CRITICAL]` = exploitable remotely with high impact, `[HIGH]` = significant risk, `[MEDIUM]` = moderate risk or requires preconditions, `[LOW]` = defense-in-depth or informational.

---

## Table of Contents

1. [OWASP Top 10 (2021)](#1-owasp-top-10-2021)
2. [XSS Deep Dive](#2-xss-deep-dive)
3. [Common Backdoors and Supply Chain](#3-common-backdoors-and-supply-chain)
4. [Secret Management](#4-secret-management)
5. [Network and Transport Security](#5-network-and-transport-security)
6. [Authentication and Session Management](#6-authentication-and-session-management)
7. [API Security](#7-api-security)
8. [Frontend-Specific Security](#8-frontend-specific-security)
9. [Infrastructure and Deployment](#9-infrastructure-and-deployment)
10. [Low-Hanging Fruit Checklist](#10-low-hanging-fruit-checklist)
11. [KMD-Specific Findings](#11-kmd-specific-findings)

---

## 1. OWASP Top 10 (2021)

### A01: Broken Access Control

| Ref | OWASP A01:2021, CWE-284 |
|-----|--------------------------|

- [ ] `[CRITICAL]` **Missing authentication on sensitive endpoints** (CWE-306)
  - **Check:** Every API endpoint that reads/writes sensitive data requires authentication. Search for route definitions without auth middleware.
  - **Attack:** Unauthenticated attacker directly calls admin or internal APIs.
  - **Fix:** Apply authentication middleware to all non-public routes. Use allowlists, not denylists.

- [ ] `[CRITICAL]` **Insecure Direct Object Reference (IDOR)** (CWE-639)
  - **Check:** API endpoints that accept user-supplied IDs (e.g., `/api/users/123/profile`). Verify the authenticated user is authorized to access that specific resource.
  - **Attack:** Attacker changes ID in URL/body to access another user's data.
  - **Fix:** Always check `resource.owner_id === authenticated_user.id` or use role-based access control. Never rely on obscurity of IDs.

- [ ] `[CRITICAL]` **Path traversal / directory traversal** (CWE-22)
  - **Check:** Any endpoint that accepts a file path or filename. Look for patterns like `req.params.filename` used in `fs.readFile()` or similar.
  - **Attack:** Attacker sends `../../etc/passwd` to read arbitrary files.
  - **Fix:** Canonicalize the path, then verify it starts with the expected base directory. Reject paths containing `..`, null bytes, or absolute paths.

- [ ] `[HIGH]` **Privilege escalation (vertical)** (CWE-269)
  - **Check:** Role changes, admin-only actions. Verify server-side role checks on every request, not just UI hiding.
  - **Attack:** Normal user modifies request to perform admin actions (e.g., changing `role: "admin"` in a PUT request).
  - **Fix:** Enforce role checks server-side on every request. Never trust client-supplied role values.

- [ ] `[HIGH]` **Privilege escalation (horizontal)** (CWE-639)
  - **Check:** User A can access or modify User B's resources by changing IDs or parameters.
  - **Attack:** Attacker enumerates or guesses resource IDs belonging to other users.
  - **Fix:** Scope all database queries to the authenticated user. Use UUIDs instead of sequential IDs.

- [ ] `[HIGH]` **CORS misconfiguration** (CWE-942)
  - **Check:** Inspect `Access-Control-Allow-Origin` headers. Look for wildcard `*` with credentials, reflected Origin, or overly permissive regex.
  - **Attack:** Malicious site makes authenticated cross-origin requests to your API and reads the response.
  - **Fix:** Allowlist specific trusted origins. Never reflect arbitrary Origin headers. Never use `*` with `credentials: true`.

- [ ] `[HIGH]` **Missing function-level access control** (CWE-285)
  - **Check:** Admin routes, management endpoints, internal tools. Verify they check authorization, not just authentication.
  - **Attack:** Authenticated low-privilege user accesses admin-only endpoints.
  - **Fix:** Implement role-based middleware that checks permissions before handler execution.

- [ ] `[MEDIUM]` **Forced browsing to unauthorized pages** (CWE-425)
  - **Check:** Try accessing URLs for admin panels, debug endpoints, or user management without proper credentials.
  - **Attack:** Attacker guesses or discovers hidden URLs (e.g., `/admin`, `/debug`, `/api/internal/`).
  - **Fix:** All endpoints must have server-side auth checks. Do not rely on URL obscurity.

- [ ] `[MEDIUM]` **Metadata manipulation** (CWE-639)
  - **Check:** JWT tokens, cookies, hidden fields, or headers that contain user identity or role information.
  - **Attack:** Attacker modifies JWT claims, cookie values, or hidden form fields to escalate privileges.
  - **Fix:** Sign and verify all tokens server-side. Never trust client-supplied metadata for authorization decisions.

- [ ] `[MEDIUM]` **Missing access control on file uploads** (CWE-434)
  - **Check:** Upload endpoints. Verify file type validation, size limits, and that uploaded files are not served with executable content types.
  - **Attack:** Attacker uploads a malicious file (web shell, HTML with scripts) and accesses it via the server.
  - **Fix:** Validate file type server-side (not just extension). Store uploads outside webroot. Serve with `Content-Disposition: attachment`. Scan for malware.

- [ ] `[LOW]` **Missing re-authentication for sensitive actions** (CWE-306)
  - **Check:** Password changes, email changes, account deletion, payment modifications. Should require current password or MFA re-verification.
  - **Attack:** Attacker with stolen session performs irreversible actions.
  - **Fix:** Require re-authentication (password or MFA) for high-impact actions.

### A02: Cryptographic Failures

| Ref | OWASP A02:2021, CWE-310 |
|-----|--------------------------|

- [ ] `[CRITICAL]` **Plaintext storage of passwords** (CWE-256)
  - **Check:** Database schema and user creation code. Grep for password storage without hashing.
  - **Attack:** Database breach exposes all user passwords.
  - **Fix:** Hash passwords with bcrypt (cost >= 12), argon2id, or scrypt. Never MD5, SHA1, or SHA256 alone.

- [ ] `[CRITICAL]` **Hardcoded secrets in source code** (CWE-798)
  - **Check:** Grep for API keys, passwords, tokens, private keys in source files. Check for patterns like `password = "..."`, `apiKey: "..."`, `SECRET_KEY = "..."`.
  - **Attack:** Anyone with source access (including via leaked repos) gets all credentials.
  - **Fix:** Use environment variables or a secrets manager (Vault, AWS Secrets Manager, etc.). Never commit secrets.

- [ ] `[CRITICAL]` **Exposed API keys in frontend code** (CWE-200)
  - **Check:** Inspect built JavaScript bundles for API keys, secret tokens, or private credentials. Search for `process.env.` patterns that may leak server secrets to the client bundle.
  - **Attack:** Anyone inspecting the browser bundle extracts API keys.
  - **Fix:** Only expose public/client-safe keys to the frontend. Keep secret keys server-side only. Use `NEXT_PUBLIC_` or `VITE_` prefixes intentionally.

- [ ] `[HIGH]` **Weak hashing algorithms for passwords** (CWE-328)
  - **Check:** Look for MD5, SHA1, SHA256 used for password hashing (without a salt or KDF).
  - **Attack:** Precomputed rainbow tables or GPU-accelerated brute force cracks passwords.
  - **Fix:** Use bcrypt, argon2id, or scrypt. These are intentionally slow and include salting.

- [ ] `[HIGH]` **Missing TLS/HTTPS** (CWE-319)
  - **Check:** Verify all production traffic uses HTTPS. Check for HTTP endpoints, mixed content, or TLS termination misconfigurations.
  - **Attack:** Network attacker (MITM) reads or modifies traffic in transit.
  - **Fix:** Enforce HTTPS everywhere. Use HSTS. Redirect HTTP to HTTPS. Use TLS 1.2+ only.

- [ ] `[HIGH]` **Secrets in URLs** (CWE-598)
  - **Check:** Look for API keys, tokens, or passwords passed as query parameters (e.g., `?api_key=xxx`). Check server logs for URL-logged secrets.
  - **Attack:** URLs are logged in browser history, server logs, proxy logs, and Referer headers.
  - **Fix:** Pass secrets in Authorization headers or request bodies. Never in URLs.

- [ ] `[HIGH]` **Weak random number generation** (CWE-330)
  - **Check:** Look for `Math.random()` used for tokens, session IDs, or security-critical values. In backend, check for non-cryptographic PRNGs.
  - **Attack:** Attacker predicts tokens or session IDs.
  - **Fix:** Use `crypto.randomBytes()` (Node.js), `crypto.getRandomValues()` (browser), or `uuid::Uuid::new_v4()` (Rust).

- [ ] `[MEDIUM]` **Sensitive data in logs** (CWE-532)
  - **Check:** Review logging statements for passwords, tokens, credit card numbers, PII. Check structured logging frameworks for auto-logged request bodies.
  - **Attack:** Anyone with log access sees sensitive data.
  - **Fix:** Scrub sensitive fields before logging. Use allowlists for loggable fields.

- [ ] `[MEDIUM]` **Insufficient key length** (CWE-326)
  - **Check:** RSA keys < 2048 bits, ECDSA keys < 256 bits, AES keys < 128 bits, HMAC secrets < 256 bits.
  - **Attack:** Brute-force or factoring attacks on short keys.
  - **Fix:** RSA >= 2048, ECDSA >= P-256, AES >= 128, HMAC >= 256 bits.

- [ ] `[MEDIUM]` **Missing encryption at rest** (CWE-311)
  - **Check:** Database, file storage, backups. Are sensitive fields (SSN, credit cards, health data) encrypted?
  - **Attack:** Physical access or backup theft exposes data.
  - **Fix:** Encrypt sensitive columns or use full-disk encryption. Manage keys separately from data.

- [ ] `[LOW]` **Deprecated TLS versions** (CWE-327)
  - **Check:** Server supports TLS 1.0 or 1.1. Test with `nmap --script ssl-enum-ciphers` or ssllabs.com.
  - **Attack:** Known protocol downgrade attacks (POODLE, BEAST).
  - **Fix:** Disable TLS 1.0 and 1.1. Support only TLS 1.2 and 1.3.

### A03: Injection

| Ref | OWASP A03:2021, CWE-74 |
|-----|--------------------------|

- [ ] `[CRITICAL]` **SQL Injection** (CWE-89)
  - **Check:** Look for string concatenation in SQL queries: `` `SELECT * FROM users WHERE id = ${id}` ``. Search for raw query calls without parameterized statements.
  - **Attack:** Attacker injects SQL to read, modify, or delete arbitrary data. Can escalate to OS command execution.
  - **Fix:** Use parameterized queries / prepared statements exclusively. Use an ORM's query builder. Never concatenate user input into SQL.

- [ ] `[CRITICAL]` **OS Command Injection** (CWE-78)
  - **Check:** Look for `exec()`, `spawn()`, `system()`, `child_process`, `subprocess`, `os.system()` with user-controlled input. Search for shell=True in Python.
  - **Attack:** Attacker injects shell metacharacters (`;`, `|`, `&&`, `` ` ``) to run arbitrary commands.
  - **Fix:** Avoid shell execution entirely. Use parameterized APIs (e.g., `spawn` with array args, not string). Allowlist valid inputs.

- [ ] `[CRITICAL]` **Stored XSS** (CWE-79)
  - **Check:** User-supplied content stored in DB and rendered to other users (comments, profiles, messages). Check for unescaped output in templates.
  - **Attack:** Attacker stores malicious script that executes in every viewer's browser, stealing sessions or performing actions.
  - **Fix:** Escape all output by default (context-aware encoding). Use CSP. Sanitize HTML input with a proven library (DOMPurify, ammonia).

- [ ] `[CRITICAL]` **Reflected XSS** (CWE-79)
  - **Check:** URL parameters, search queries, error messages reflected in HTML without encoding. Test with `<script>alert(1)</script>` in all input fields.
  - **Attack:** Attacker crafts a URL that executes script in the victim's browser.
  - **Fix:** Encode all reflected output. Use CSP. Set `X-Content-Type-Options: nosniff`.

- [ ] `[HIGH]` **DOM-based XSS** (CWE-79)
  - **Check:** JavaScript that reads from `location.hash`, `location.search`, `document.referrer`, `window.name`, `postMessage`, or `localStorage` and writes to `innerHTML`, `document.write()`, `eval()`, or `$.html()`.
  - **Attack:** Attacker controls DOM sources and triggers script execution through DOM sinks.
  - **Fix:** Use `textContent` instead of `innerHTML`. Parse URLs with `URL()` API. Validate all DOM-sourced data.

- [ ] `[HIGH]` **Server-Side Template Injection (SSTI)** (CWE-94)
  - **Check:** User input rendered in server-side templates (Jinja2, Twig, EJS, Handlebars). Test with `{{7*7}}` or `${7*7}`.
  - **Attack:** Attacker achieves remote code execution through template evaluation.
  - **Fix:** Never pass user input to template engines as template code. Use sandboxed template engines. Treat user input as data, not templates.

- [ ] `[HIGH]` **LDAP Injection** (CWE-90)
  - **Check:** LDAP queries constructed with user input. Look for string concatenation in LDAP filter expressions.
  - **Attack:** Attacker modifies LDAP queries to bypass authentication or read unauthorized data.
  - **Fix:** Use parameterized LDAP APIs. Escape special characters (`*`, `(`, `)`, `\`, NUL).

- [ ] `[HIGH]` **Header Injection / HTTP Response Splitting** (CWE-113)
  - **Check:** User input written into HTTP response headers (redirects, Set-Cookie, custom headers). Look for `\r\n` injection.
  - **Attack:** Attacker injects headers to set cookies, redirect users, or create XSS through response splitting.
  - **Fix:** Validate and sanitize all values written to headers. Strip `\r` and `\n` characters.

- [ ] `[MEDIUM]` **Log Injection** (CWE-117)
  - **Check:** User input written to log files without sanitization. Look for `logger.info(userInput)` patterns.
  - **Attack:** Attacker injects fake log entries or ANSI escape codes to confuse log analysis or exploit log viewers.
  - **Fix:** Sanitize log output: strip newlines, escape special characters, use structured logging (JSON).

- [ ] `[MEDIUM]` **NoSQL Injection** (CWE-943)
  - **Check:** MongoDB queries with user input: `db.users.find({ username: req.body.username })`. Test with `{"$gt": ""}`.
  - **Attack:** Attacker injects query operators to bypass authentication or extract data.
  - **Fix:** Validate input types. Reject objects where strings are expected. Use schema validation.

- [ ] `[MEDIUM]` **XPath Injection** (CWE-643)
  - **Check:** XML processing with user-controlled XPath expressions.
  - **Attack:** Attacker modifies XPath queries to extract unauthorized XML data.
  - **Fix:** Use parameterized XPath APIs. Validate input against expected patterns.

- [ ] `[MEDIUM]` **Email Header Injection** (CWE-93)
  - **Check:** User input in email headers (To, CC, BCC, Subject). Look for `\r\n` in email fields.
  - **Attack:** Attacker injects additional headers to send spam, add recipients, or modify email content.
  - **Fix:** Validate email addresses. Strip newlines from all header fields.

### A04: Insecure Design

| Ref | OWASP A04:2021, CWE-840 |
|-----|--------------------------|

- [ ] `[HIGH]` **Missing rate limiting on critical endpoints** (CWE-770)
  - **Check:** Login, signup, password reset, OTP verification, API endpoints. Can an attacker make unlimited requests?
  - **Attack:** Brute-force attacks, credential stuffing, denial of service, resource exhaustion.
  - **Fix:** Implement rate limiting (token bucket, sliding window). Use progressive delays. Add CAPTCHA after threshold.

- [ ] `[HIGH]` **Business logic flaws** (CWE-840)
  - **Check:** Order flows, payment processing, coupon application, voting systems. Can steps be skipped, repeated, or reordered?
  - **Attack:** Attacker manipulates multi-step processes (e.g., applying a discount twice, skipping payment).
  - **Fix:** Enforce state machine transitions server-side. Validate every step. Use idempotency keys for financial operations.

- [ ] `[HIGH]` **Missing input validation** (CWE-20)
  - **Check:** All user inputs -- form fields, API parameters, file uploads, headers. Are types, ranges, lengths, and formats validated server-side?
  - **Attack:** Unexpected input causes crashes, injection, or logic bypass.
  - **Fix:** Validate all input server-side with strict schemas. Use allowlists over denylists. Validate at system boundaries.

- [ ] `[MEDIUM]` **No abuse case analysis** (CWE-840)
  - **Check:** Has the team considered how each feature could be abused? Are there user stories for malicious actors?
  - **Attack:** Features designed only for happy paths get exploited in unexpected ways.
  - **Fix:** For each feature, document abuse cases alongside user stories. Implement mitigations proactively.

- [ ] `[MEDIUM]` **Insufficient anti-automation** (CWE-799)
  - **Check:** Account creation, contact forms, comment submission. Can bots abuse these at scale?
  - **Attack:** Mass account creation, spam, scraping, vote manipulation.
  - **Fix:** CAPTCHA, rate limiting, proof-of-work challenges, behavioral analysis.

- [ ] `[MEDIUM]` **Race conditions in business logic** (CWE-362)
  - **Check:** Concurrent requests to the same resource (balance transfers, inventory, coupon redemption). Test with parallel requests.
  - **Attack:** Attacker sends simultaneous requests to exploit TOCTOU (time-of-check-to-time-of-use) bugs.
  - **Fix:** Use database transactions with appropriate isolation levels. Use optimistic locking. Use idempotency keys.

- [ ] `[LOW]` **Missing account lockout** (CWE-307)
  - **Check:** After N failed login attempts, is the account locked or rate-limited?
  - **Attack:** Unlimited login attempts enable brute force.
  - **Fix:** Lock account after 5-10 failures. Require CAPTCHA. Implement progressive delays. Notify user.

### A05: Security Misconfiguration

| Ref | OWASP A05:2021, CWE-16 |
|-----|--------------------------|

- [ ] `[CRITICAL]` **Default credentials** (CWE-1393)
  - **Check:** Admin panels, databases, message queues, monitoring tools. Are default usernames/passwords still active?
  - **Attack:** Attacker logs in with well-known default credentials (admin/admin, root/root).
  - **Fix:** Force password change on first login. Remove or disable default accounts. Automate credential rotation.

- [ ] `[CRITICAL]` **Debug mode enabled in production** (CWE-489)
  - **Check:** Framework debug settings (`DEBUG=True`, `NODE_ENV=development`, `RAILS_ENV=development`). Check for exposed stack traces, source maps, debug endpoints.
  - **Attack:** Debug mode leaks source code, environment variables, database queries, and internal paths.
  - **Fix:** Ensure production deployments set `NODE_ENV=production`, `DEBUG=false`, etc. Automate environment checks in CI/CD.

- [ ] `[HIGH]` **Verbose error messages** (CWE-209)
  - **Check:** Trigger errors (404, 500, validation errors). Do responses include stack traces, SQL queries, file paths, or internal details?
  - **Attack:** Error details reveal internal architecture, database schema, library versions.
  - **Fix:** Return generic error messages to clients. Log detailed errors server-side only.

- [ ] `[HIGH]` **Missing security headers** (CWE-693)
  - **Check:** Verify presence of: `Content-Security-Policy`, `X-Content-Type-Options`, `X-Frame-Options`, `Strict-Transport-Security`, `Referrer-Policy`, `Permissions-Policy`.
  - **Attack:** Missing headers enable XSS, clickjacking, MIME sniffing, information leakage.
  - **Fix:** Add all security headers. Use a middleware or reverse proxy to apply them globally.

- [ ] `[HIGH]` **Permissive CORS configuration** (CWE-942)
  - **Check:** `Access-Control-Allow-Origin: *` with `Access-Control-Allow-Credentials: true`. Reflected Origin without validation.
  - **Attack:** Any website can make authenticated requests and read responses.
  - **Fix:** Allowlist specific origins. Never reflect arbitrary origins with credentials.

- [ ] `[HIGH]` **Unnecessary features enabled** (CWE-1188)
  - **Check:** Unused HTTP methods (TRACE, OPTIONS responding broadly), directory listing, WebDAV, server-status pages.
  - **Attack:** Exposed features increase attack surface.
  - **Fix:** Disable all features not required in production. Restrict HTTP methods to those actually used.

- [ ] `[MEDIUM]` **Exposed server version information** (CWE-200)
  - **Check:** `Server` header, `X-Powered-By` header, framework-specific cookies or headers.
  - **Attack:** Version information helps attacker find known CVEs for that specific version.
  - **Fix:** Remove or genericize version headers: `Server: webserver`. Remove `X-Powered-By`.

- [ ] `[MEDIUM]` **Source maps in production** (CWE-540)
  - **Check:** Are `.map` files deployed to production? Can browser DevTools load source maps?
  - **Attack:** Source maps reveal original source code, making it easier to find vulnerabilities.
  - **Fix:** Do not deploy source maps to production. If needed for error tracking, use private source map upload to your error service.

- [ ] `[MEDIUM]` **Exposed .git directory** (CWE-538)
  - **Check:** Try accessing `/.git/HEAD`, `/.git/config` on the production domain.
  - **Attack:** Attacker reconstructs entire source code repository.
  - **Fix:** Block access to `.git/` in your web server or CDN config.

- [ ] `[LOW]` **Missing Permissions-Policy header** (CWE-693)
  - **Check:** Is `Permissions-Policy` (formerly Feature-Policy) set to restrict APIs like camera, microphone, geolocation?
  - **Attack:** Third-party scripts or embedded content access sensitive browser APIs.
  - **Fix:** Set `Permissions-Policy: camera=(), microphone=(), geolocation=()` etc.

### A06: Vulnerable and Outdated Components

| Ref | OWASP A06:2021, CWE-1104 |
|-----|--------------------------|

- [ ] `[CRITICAL]` **Known CVEs in dependencies** (CWE-1035)
  - **Check:** Run `npm audit`, `cargo audit`, `pip audit`, `snyk test`, or `trivy` against the project.
  - **Attack:** Attacker exploits known vulnerabilities in unpatched libraries.
  - **Fix:** Update dependencies regularly. Set up automated scanning in CI (Dependabot, Snyk, Renovate).

- [ ] `[HIGH]` **Outdated framework or runtime** (CWE-1104)
  - **Check:** Node.js, Python, Rust, Go, Java version. Is it within the supported/LTS window?
  - **Attack:** EOL runtimes no longer receive security patches.
  - **Fix:** Use LTS versions. Upgrade regularly. Track EOL dates.

- [ ] `[HIGH]` **Unmaintained dependencies** (CWE-1104)
  - **Check:** Check last commit date, open issues, and maintainer activity for each dependency.
  - **Attack:** Unmaintained packages may have unpatched vulnerabilities or get hijacked.
  - **Fix:** Replace unmaintained packages with actively maintained alternatives. Minimize dependency count.

- [ ] `[MEDIUM]` **Missing lock file** (CWE-1104)
  - **Check:** Verify `package-lock.json`, `Cargo.lock`, `poetry.lock`, etc. is committed and used in CI.
  - **Attack:** Without a lock file, builds may pull different (potentially compromised) versions.
  - **Fix:** Commit lock files. Use `npm ci` (not `npm install`) in CI.

- [ ] `[MEDIUM]` **Nested/transitive dependency vulnerabilities** (CWE-1035)
  - **Check:** `npm audit` includes transitive dependencies. Check the full dependency tree.
  - **Attack:** Vulnerability deep in the dependency tree is still exploitable.
  - **Fix:** Use `npm audit fix` or override vulnerable transitive deps. Consider `npm-force-resolutions` or `overrides`.

- [ ] `[LOW]` **No automated dependency update process** (CWE-1104)
  - **Check:** Is there a Dependabot/Renovate config? Are PRs reviewed and merged regularly?
  - **Attack:** Dependencies drift and accumulate vulnerabilities over time.
  - **Fix:** Enable automated dependency PRs. Review and merge weekly.

### A07: Identification and Authentication Failures

| Ref | OWASP A07:2021, CWE-287 |
|-----|--------------------------|

- [ ] `[CRITICAL]` **Missing authentication on sensitive endpoints** (CWE-306)
  - **Check:** List all API routes. Verify each sensitive route requires valid authentication.
  - **Attack:** Unauthenticated access to user data, admin functions, or internal APIs.
  - **Fix:** Apply auth middleware globally. Explicitly mark public endpoints.

- [ ] `[CRITICAL]` **Brute-force login** (CWE-307)
  - **Check:** Can an attacker make unlimited login attempts? Test with automated tools.
  - **Attack:** Attacker tries millions of password combinations.
  - **Fix:** Rate limit login attempts per IP and per account. Add CAPTCHA after failures. Implement account lockout.

- [ ] `[HIGH]` **Credential stuffing** (CWE-521)
  - **Check:** Is there protection against automated login with leaked credential lists?
  - **Attack:** Attacker uses credentials from data breaches to log into accounts.
  - **Fix:** Rate limiting, CAPTCHA, breached password detection (Have I Been Pwned API), MFA.

- [ ] `[HIGH]` **Weak password policy** (CWE-521)
  - **Check:** What are the minimum password requirements? Are common passwords blocked?
  - **Attack:** Users choose weak passwords that are easily guessed.
  - **Fix:** Minimum 8 characters, check against common password lists (top 10K), encourage passphrases. Use zxcvbn or similar strength estimator.

- [ ] `[HIGH]` **Session fixation** (CWE-384)
  - **Check:** After login, is a new session ID generated? Does the old session ID still work?
  - **Attack:** Attacker sets a known session ID before login, then hijacks the session after the user authenticates.
  - **Fix:** Regenerate session ID on login. Invalidate old sessions.

- [ ] `[HIGH]` **Missing multi-factor authentication** (CWE-308)
  - **Check:** Is MFA available for all users? Is it enforced for admin accounts?
  - **Attack:** Stolen password alone grants full access.
  - **Fix:** Implement TOTP or WebAuthn MFA. Enforce for admin and high-privilege accounts.

- [ ] `[MEDIUM]` **Insecure password reset** (CWE-640)
  - **Check:** Reset token entropy, expiration, single-use enforcement. Is the token in the URL?
  - **Attack:** Predictable or reusable reset tokens let attacker take over accounts.
  - **Fix:** Use high-entropy tokens (>= 128 bits). Expire in 15-30 minutes. Single-use. Send via secure channel.

- [ ] `[MEDIUM]` **Missing session timeout** (CWE-613)
  - **Check:** How long do sessions last? Is there an idle timeout? Absolute timeout?
  - **Attack:** Stale sessions on shared/public computers remain active indefinitely.
  - **Fix:** Idle timeout (15-30 min for sensitive apps). Absolute timeout (8-24 hours). Provide logout.

- [ ] `[MEDIUM]` **User enumeration** (CWE-204)
  - **Check:** Does the login page reveal whether a username exists? ("Invalid password" vs "User not found"). Check registration, password reset too.
  - **Attack:** Attacker builds a list of valid usernames for targeted attacks.
  - **Fix:** Use consistent error messages: "Invalid credentials". Use consistent timing (prevent timing-based enumeration).

- [ ] `[LOW]` **Missing session invalidation on password change** (CWE-613)
  - **Check:** After changing password, are all other active sessions invalidated?
  - **Attack:** Attacker's session remains active even after the real user changes their password.
  - **Fix:** Invalidate all sessions on password change. Optionally notify user of active sessions.

### A08: Software and Data Integrity Failures

| Ref | OWASP A08:2021, CWE-502 |
|-----|--------------------------|

- [ ] `[CRITICAL]` **Insecure deserialization** (CWE-502)
  - **Check:** Look for `JSON.parse()` on untrusted data with prototype pollution potential, `pickle.loads()`, `ObjectInputStream`, `unserialize()`, or YAML `load()` (vs `safe_load()`).
  - **Attack:** Attacker crafts malicious serialized data to achieve remote code execution or object injection.
  - **Fix:** Use safe deserialization methods. Validate and type-check deserialized data. Avoid native serialization formats for untrusted data.

- [ ] `[HIGH]` **Missing Subresource Integrity (SRI)** (CWE-353)
  - **Check:** External scripts and stylesheets loaded from CDNs. Do `<script>` and `<link>` tags have `integrity` attributes?
  - **Attack:** Compromised CDN serves malicious JavaScript to all users.
  - **Fix:** Add `integrity="sha384-..."` and `crossorigin="anonymous"` to all external resource tags.

- [ ] `[HIGH]` **Unsigned or unverified updates** (CWE-494)
  - **Check:** Auto-update mechanisms, plugin installations, package downloads. Are signatures verified?
  - **Attack:** Attacker intercepts update and injects malicious code.
  - **Fix:** Sign all releases. Verify signatures before installation. Use HTTPS for downloads.

- [ ] `[HIGH]` **Insecure CI/CD pipeline** (CWE-829)
  - **Check:** Can PRs from forks run CI with secrets? Are CI scripts audited? Are build artifacts signed?
  - **Attack:** Attacker submits PR that exfiltrates CI secrets or modifies build output.
  - **Fix:** Restrict secret access to protected branches. Require approval for fork CI runs. Sign artifacts. Audit CI configs.

- [ ] `[MEDIUM]` **Prototype pollution (JavaScript)** (CWE-1321)
  - **Check:** Deep merge utilities, `Object.assign()` with user-controlled keys, lodash `_.merge()` with untrusted data.
  - **Attack:** Attacker pollutes `Object.prototype` to modify application behavior (XSS, privilege escalation).
  - **Fix:** Use `Object.create(null)` for dictionaries. Validate keys (reject `__proto__`, `constructor`, `prototype`). Use `Map` instead of plain objects.

- [ ] `[MEDIUM]` **Missing code signing** (CWE-353)
  - **Check:** Are release binaries, Docker images, or packages signed?
  - **Attack:** Tampered binaries distributed to users.
  - **Fix:** Sign releases with GPG or Sigstore. Publish checksums. Use Docker Content Trust.

### A09: Security Logging and Monitoring Failures

| Ref | OWASP A09:2021, CWE-778 |
|-----|--------------------------|

- [ ] `[HIGH]` **No audit trail for security events** (CWE-778)
  - **Check:** Are login attempts, access denials, privilege changes, data modifications logged?
  - **Attack:** Attacker operates undetected; incidents cannot be investigated.
  - **Fix:** Log all authentication events, authorization failures, input validation failures, and data changes. Include timestamp, user ID, IP, action, and result.

- [ ] `[HIGH]` **Sensitive data in logs** (CWE-532)
  - **Check:** Search logs for passwords, tokens, credit card numbers, SSNs, session IDs.
  - **Attack:** Log access (by ops, SIEM, or attacker) exposes sensitive data.
  - **Fix:** Never log passwords, tokens, or PII. Mask sensitive fields. Use structured logging with field-level redaction.

- [ ] `[MEDIUM]` **Missing alerting on security events** (CWE-778)
  - **Check:** Are there alerts for: brute-force attempts, admin login from new IP, mass data export, unusual error rates?
  - **Attack:** Active attack goes unnoticed until damage is done.
  - **Fix:** Set up alerts in SIEM/monitoring for anomalous patterns. Define runbooks for incident response.

- [ ] `[MEDIUM]` **Insufficient log retention** (CWE-779)
  - **Check:** How long are logs retained? Is retention sufficient for incident investigation (typically 90 days minimum)?
  - **Attack:** Evidence is lost before an incident is detected.
  - **Fix:** Retain security logs for 90-365 days. Use immutable/append-only log storage.

- [ ] `[MEDIUM]` **Logs not tamper-proof** (CWE-117)
  - **Check:** Can an attacker with server access modify or delete logs?
  - **Attack:** Attacker covers tracks by modifying log files.
  - **Fix:** Ship logs to a remote, append-only service (CloudWatch, Datadog, Splunk). Use log integrity verification.

- [ ] `[LOW]` **No logging of admin actions** (CWE-778)
  - **Check:** Are all admin actions (user management, config changes, deployments) logged with who/what/when?
  - **Attack:** Rogue admin or compromised admin account acts undetected.
  - **Fix:** Log all admin actions to a tamper-proof audit trail.

### A10: Server-Side Request Forgery (SSRF)

| Ref | OWASP A10:2021, CWE-918 |
|-----|--------------------------|

- [ ] `[CRITICAL]` **SSRF to internal services** (CWE-918)
  - **Check:** Any endpoint that fetches a URL provided by the user (webhooks, URL preview, file import, image proxy). Can the user specify `http://localhost:*`, `http://127.0.0.1:*`, or `http://169.254.169.254/*`?
  - **Attack:** Attacker accesses internal services, databases, or cloud metadata (AWS IAM credentials).
  - **Fix:** Validate and allowlist target URLs. Block private/reserved IP ranges. Use a URL parser, not regex. Disable redirects or validate each hop.

- [ ] `[CRITICAL]` **Cloud metadata access via SSRF** (CWE-918)
  - **Check:** Can the application be tricked into requesting `http://169.254.169.254/latest/meta-data/` (AWS), `http://metadata.google.internal/` (GCP), or `http://169.254.169.254/metadata/instance` (Azure)?
  - **Attack:** Attacker extracts cloud IAM credentials, enabling full account takeover.
  - **Fix:** Block requests to `169.254.169.254` and other cloud metadata IPs. Use IMDSv2 (AWS) which requires headers. Use network-level controls.

- [ ] `[HIGH]` **Protocol smuggling via SSRF** (CWE-918)
  - **Check:** Can the user-supplied URL use protocols other than HTTP/HTTPS? Test with `file://`, `gopher://`, `dict://`, `ftp://`.
  - **Attack:** Non-HTTP protocols can interact with internal services (Redis, Memcached) or read local files.
  - **Fix:** Allowlist only `http://` and `https://` protocols. Validate scheme before making the request.

- [ ] `[HIGH]` **SSRF via DNS rebinding** (CWE-918)
  - **Check:** Does the application resolve DNS, validate the IP, then make a separate request? The DNS could change between checks.
  - **Attack:** DNS resolves to a safe IP during validation but to an internal IP during the actual request.
  - **Fix:** Resolve DNS once and use the resolved IP for the request. Pin the DNS resolution. Use a safe HTTP library that prevents rebinding.

- [ ] `[MEDIUM]` **Blind SSRF** (CWE-918)
  - **Check:** Even if the response is not returned to the user, can the attacker infer information from timing, error messages, or side effects?
  - **Attack:** Attacker maps internal network by observing response times or error differences.
  - **Fix:** Apply the same URL validation as for full SSRF. Monitor for unusual outbound requests.

---

## 2. XSS Deep Dive

### DOM Sinks and Sources

- [ ] `[CRITICAL]` **innerHTML assignment with user data** (CWE-79)
  - **Check:** Grep for `.innerHTML =`, `.innerHTML +=`, `.outerHTML =` where the assigned value includes any user-controlled data.
  - **Attack:** Attacker injects `<img src=x onerror=alert(1)>` or `<svg onload=...>` through user-controlled data.
  - **Fix:** Use `.textContent` for text. Use DOM APIs (`createElement`, `setAttribute`) for structured content. If HTML is required, sanitize with DOMPurify.

- [ ] `[CRITICAL]` **dangerouslySetInnerHTML (React)** (CWE-79)
  - **Check:** Grep for `dangerouslySetInnerHTML`. Verify every usage sanitizes input with DOMPurify or equivalent before rendering.
  - **Attack:** Unsanitized user data in `dangerouslySetInnerHTML` enables stored/reflected XSS.
  - **Fix:** Always sanitize with DOMPurify before passing to `dangerouslySetInnerHTML`. Better yet, use React's default escaping and avoid it entirely.

- [ ] `[CRITICAL]` **document.write with user data** (CWE-79)
  - **Check:** Grep for `document.write(`, `document.writeln(`. These should never include user-controlled data.
  - **Attack:** Full DOM control -- attacker can inject any HTML/script.
  - **Fix:** Replace `document.write` with DOM APIs. This API is almost never needed in modern code.

- [ ] `[CRITICAL]` **eval() and equivalents with user data** (CWE-95)
  - **Check:** Grep for `eval(`, `new Function(`, `setTimeout(string`, `setInterval(string`. Verify none include user input.
  - **Attack:** Direct JavaScript code execution.
  - **Fix:** Remove all `eval()` usage. Use `JSON.parse()` for data. Use function references for callbacks.

- [ ] `[HIGH]` **DOM XSS via URL fragments** (CWE-79)
  - **Check:** JavaScript that reads `location.hash`, `location.search`, or `location.href` and writes to DOM sinks.
  - **Attack:** `https://app.com/#<img src=x onerror=alert(1)>` -- hash changes don't trigger server requests, so server-side protections are bypassed.
  - **Fix:** Validate and sanitize all URL-derived data before DOM insertion. Use `URL()` and `URLSearchParams()` APIs.

- [ ] `[HIGH]` **DOM XSS via localStorage/sessionStorage** (CWE-79)
  - **Check:** Data read from storage and inserted into DOM without sanitization. Search for `localStorage.getItem()` followed by `innerHTML`.
  - **Attack:** If any XSS vector can write to storage, it persists across page loads (persistent DOM XSS).
  - **Fix:** Treat storage data as untrusted. Sanitize before DOM insertion.

- [ ] `[HIGH]` **DOM XSS via postMessage** (CWE-79)
  - **Check:** `window.addEventListener('message', ...)` handlers. Is `event.origin` validated? Is `event.data` sanitized before DOM insertion?
  - **Attack:** Any page can send postMessage to your window. Without origin checks, attacker page injects data.
  - **Fix:** Always check `event.origin` against an allowlist. Sanitize `event.data` before use.

### Advanced XSS Vectors

- [ ] `[HIGH]` **SVG XSS** (CWE-79)
  - **Check:** User-uploaded SVGs or SVG content in user input. SVGs can contain `<script>`, `<foreignObject>`, and event handlers.
  - **Attack:** `<svg><script>alert(1)</script></svg>` or `<svg onload="alert(1)">`.
  - **Fix:** Sanitize SVGs by stripping `<script>`, `<foreignObject>`, and all event handler attributes. Serve user SVGs with `Content-Type: image/svg+xml` and CSP.

- [ ] `[HIGH]` **MathML XSS** (CWE-79)
  - **Check:** MathML content in user input. Some browsers execute scripts within MathML.
  - **Attack:** `<math><mtext><table><mglyph><style><!--</style><img src=x onerror=alert(1)>` -- parser differential attacks.
  - **Fix:** Strip MathML from user input if not needed. Use a sanitizer that handles MathML (DOMPurify with MathML disabled or properly configured).

- [ ] `[HIGH]` **Mutation XSS (mXSS)** (CWE-79)
  - **Check:** HTML that mutates after being parsed by the browser's HTML parser, bypassing sanitizers. Particularly in innerHTML assignments after serialization/deserialization.
  - **Attack:** Specially crafted HTML that appears safe to the sanitizer but becomes dangerous after browser mutation (e.g., backtick attributes in IE, noscript parsing differences).
  - **Fix:** Use DOMPurify (which handles mXSS). Sanitize on the server AND the client. Avoid serialization/deserialization cycles.

- [ ] `[HIGH]` **Template literal injection** (CWE-79)
  - **Check:** JavaScript template literals that include user data: `` `<div>${userInput}</div>` `` assigned to `innerHTML`.
  - **Attack:** User input in template literals that end up in DOM sinks.
  - **Fix:** Never use template literals to construct HTML. Use DOM APIs or a templating library with auto-escaping.

- [ ] `[HIGH]` **Event handler injection** (CWE-79)
  - **Check:** User input in HTML attributes that could be event handlers: `<div title="${userInput}">`. If quotes can be broken, attacker adds `onmouseover=alert(1)`.
  - **Attack:** Attribute injection leads to event handler execution.
  - **Fix:** Properly encode output for HTML attribute context. Use framework auto-escaping. Validate attribute names.

- [ ] `[HIGH]` **javascript: and data: URI schemes** (CWE-79)
  - **Check:** User-controlled URLs in `href`, `src`, `action` attributes. Test with `javascript:alert(1)` and `data:text/html,<script>alert(1)</script>`.
  - **Attack:** Clicking a `javascript:` link or loading a `data:` URI executes arbitrary code.
  - **Fix:** Allowlist URL schemes (`http:`, `https:`, `mailto:`). Block `javascript:`, `data:` (except `data:image/*` for img src), `vbscript:`.

### CSP and XSS Prevention

- [ ] `[HIGH]` **Missing Content Security Policy** (CWE-693)
  - **Check:** Is a `Content-Security-Policy` header set? Is it strict enough to prevent inline scripts?
  - **Attack:** Without CSP, any XSS vulnerability leads to full script execution.
  - **Fix:** Set a strict CSP: `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; object-src 'none'; base-uri 'self'`. Use nonces or hashes for inline scripts.

- [ ] `[HIGH]` **CSP with unsafe-inline for scripts** (CWE-693)
  - **Check:** CSP includes `script-src 'unsafe-inline'` or `script-src *`.
  - **Attack:** `unsafe-inline` completely negates XSS protection from CSP.
  - **Fix:** Remove `unsafe-inline` from `script-src`. Use nonces (`'nonce-xxx'`) or hashes (`'sha256-xxx'`) for necessary inline scripts.

- [ ] `[MEDIUM]` **CSP bypass via JSONP** (CWE-693)
  - **Check:** CSP allows a domain that serves JSONP endpoints (e.g., `script-src accounts.google.com`).
  - **Attack:** Attacker uses JSONP callback to execute arbitrary JavaScript from an allowed domain.
  - **Fix:** Use `strict-dynamic` with nonces. Avoid allowlisting domains with JSONP endpoints.

- [ ] `[MEDIUM]` **CSP bypass via base-uri** (CWE-693)
  - **Check:** CSP does not restrict `base-uri`. An attacker who can inject `<base href="https://evil.com">` changes relative URL resolution.
  - **Attack:** Relative script/link paths resolve to attacker's server.
  - **Fix:** Add `base-uri 'self'` to CSP.

- [ ] `[MEDIUM]` **CSP bypass via object-src** (CWE-693)
  - **Check:** CSP allows `object-src *` or does not set `object-src`.
  - **Attack:** Flash/Java plugins can bypass CSP restrictions.
  - **Fix:** Set `object-src 'none'`.

- [ ] `[LOW]` **CSP report-only without report-uri** (CWE-693)
  - **Check:** Is CSP in report-only mode with no `report-uri` or `report-to`? This provides zero protection and zero visibility.
  - **Attack:** CSP violations are silently ignored.
  - **Fix:** Either enforce CSP or configure report-uri to collect violations before enforcing.

---

## 3. Common Backdoors and Supply Chain

### Dependency Hijacking

- [ ] `[CRITICAL]` **Typosquatting packages** (CWE-829)
  - **Check:** Review all dependency names for typos of popular packages (e.g., `crossenv` vs `cross-env`, `event-stream` vs `events-stream`).
  - **Attack:** Attacker publishes a package with a name similar to a popular one. Developers install it by mistake.
  - **Fix:** Audit all dependencies manually. Use `npm ls` to verify packages. Pin exact versions.

- [ ] `[CRITICAL]` **Malicious postinstall scripts** (CWE-829)
  - **Check:** Run `npm ls --json | jq '.dependencies | to_entries[] | select(.value.scripts.postinstall)'` or check `node_modules/*/package.json` for `postinstall`, `preinstall`, `install` scripts.
  - **Attack:** Malicious package runs arbitrary code during `npm install`.
  - **Fix:** Use `--ignore-scripts` flag during install, then selectively run trusted scripts. Use `npm audit signatures`. Review new dependencies before adding.

- [ ] `[CRITICAL]` **Compromised maintainer accounts** (CWE-829)
  - **Check:** Monitor for unexpected new releases of dependencies (especially minor/patch bumps with large diffs).
  - **Attack:** Attacker gains access to a maintainer's npm/PyPI account and publishes a malicious version.
  - **Fix:** Pin exact dependency versions. Review diffs for dependency updates. Use lock files. Enable 2FA on your own package accounts.

- [ ] `[HIGH]` **Dependency confusion / namespace hijacking** (CWE-829)
  - **Check:** If using private registries, verify that public registries cannot serve packages with the same name. Check `.npmrc` for registry configuration.
  - **Attack:** Attacker publishes a public package with the same name as your private package. Build system pulls the public (malicious) version.
  - **Fix:** Use scoped packages (`@org/package`). Configure `.npmrc` to only use private registry for specific scopes. Use `registry` overrides.

- [ ] `[HIGH]` **Lock file manipulation** (CWE-829)
  - **Check:** Review lock file diffs in PRs. Look for changed integrity hashes, new registries, or unexpected version changes.
  - **Attack:** Attacker modifies lock file to point to a malicious package version or registry.
  - **Fix:** Review lock file changes in code review. Use `npm ci` (which respects lock file exactly). Set up CI to flag lock file changes.

### CI/CD Security

- [ ] `[CRITICAL]` **GitHub Actions injection via pull_request_target** (CWE-829)
  - **Check:** Do workflows use `pull_request_target` trigger and checkout PR code? Do they use `${{ github.event.pull_request.title }}` or similar in `run:` steps?
  - **Attack:** Attacker's PR title/body contains shell injection that executes in a privileged context with repo secrets.
  - **Fix:** Never use `pull_request_target` with PR code checkout. Use `pull_request` trigger instead. Never interpolate event data in `run:` steps -- use environment variables.

- [ ] `[HIGH]` **Secrets exposed in CI logs** (CWE-532)
  - **Check:** Are CI secrets masked in logs? Could `echo $SECRET` or error messages leak them?
  - **Attack:** Anyone with log access reads secrets.
  - **Fix:** Use CI secret masking features. Never echo secrets. Audit CI scripts for accidental exposure.

- [ ] `[HIGH]` **Unrestricted CI on forks** (CWE-829)
  - **Check:** Can PRs from forked repos trigger CI workflows that have access to secrets?
  - **Attack:** Attacker forks repo, modifies CI workflow, and exfiltrates secrets.
  - **Fix:** Require approval for fork CI runs. Do not expose secrets to fork PRs. Use `pull_request` (not `pull_request_target`).

- [ ] `[MEDIUM]` **Mutable action references** (CWE-829)
  - **Check:** GitHub Actions using `@main` or `@v1` (branch/tag refs) instead of pinned SHA hashes.
  - **Attack:** Action maintainer pushes malicious update to a tag; all users get the malicious version.
  - **Fix:** Pin actions to full commit SHAs: `uses: actions/checkout@a81bbb...`. Use Dependabot to update action pins.

---

## 4. Secret Management

### Secrets in Code and Storage

- [ ] `[CRITICAL]` **API keys in frontend source** (CWE-200)
  - **Check:** Grep built JavaScript bundles for patterns: API keys, `sk_live_`, `AKIA`, `ghp_`, `Bearer `, base64-encoded secrets.
  - **Attack:** Anyone viewing page source or DevTools extracts secret API keys.
  - **Fix:** Keep all secret keys server-side. Only expose publishable/public keys to the frontend. Use a backend proxy for API calls requiring secrets.

- [ ] `[CRITICAL]` **Secrets in git history** (CWE-540)
  - **Check:** Run `git log --all -p | grep -i "password\|secret\|api_key\|token"` or use tools like `trufflehog`, `gitleaks`, `git-secrets`.
  - **Attack:** Even if a secret is removed from HEAD, it remains in git history forever.
  - **Fix:** Rotate any leaked secrets immediately. Use `git filter-repo` or `BFG Repo Cleaner` to remove from history (force push required). Add pre-commit hooks (`git-secrets`, `detect-secrets`).

- [ ] `[CRITICAL]` **.env files committed to repository** (CWE-538)
  - **Check:** Check for `.env`, `.env.local`, `.env.production` in the git repo. Verify `.gitignore` excludes them.
  - **Attack:** All secrets in the .env file are exposed to anyone with repo access.
  - **Fix:** Add `.env*` to `.gitignore` (except `.env.example`). Audit git history for past commits. Rotate exposed secrets.

- [ ] `[CRITICAL]` **Hardcoded credentials** (CWE-798)
  - **Check:** Grep for `password`, `secret`, `apikey`, `token`, `credential` in source files. Look for string literals assigned to these variables.
  - **Attack:** Credentials are visible to anyone with code access and cannot be rotated without code changes.
  - **Fix:** Use environment variables or a secrets manager. Never hardcode credentials.

- [ ] `[HIGH]` **Secrets in URLs (query parameters)** (CWE-598)
  - **Check:** Look for API calls with tokens in query strings: `fetch('/api?token=xxx')`, `<img src="/image?key=xxx">`.
  - **Attack:** URLs are logged in server access logs, browser history, proxy logs, Referer headers, and analytics.
  - **Fix:** Pass secrets in `Authorization` header or POST body. Never in URLs.

- [ ] `[HIGH]` **Secrets in localStorage/sessionStorage** (CWE-922)
  - **Check:** Grep for `localStorage.setItem` or `sessionStorage.setItem` with token/key values.
  - **Attack:** XSS can read all localStorage/sessionStorage data. They are accessible to all scripts on the origin.
  - **Fix:** Store tokens in httpOnly cookies (not accessible to JavaScript). If localStorage is required, minimize token lifetime and scope.

- [ ] `[HIGH]` **Secrets in error messages** (CWE-209)
  - **Check:** Trigger errors with invalid API keys, database connection strings, etc. Do error messages include the secret?
  - **Attack:** Error messages shown to users or logged with secrets expose them.
  - **Fix:** Never include secrets in error messages. Log them separately with redaction.

- [ ] `[HIGH]` **Secrets in Docker image layers** (CWE-538)
  - **Check:** Use `docker history --no-trunc <image>` or `dive` to inspect all layers. Secrets added then deleted still exist in prior layers.
  - **Attack:** Anyone who pulls the image can inspect all layers and extract secrets.
  - **Fix:** Use multi-stage builds. Use Docker secrets or BuildKit secret mounts (`--mount=type=secret`). Never COPY secrets into intermediate layers.

- [ ] `[MEDIUM]` **Secrets in browser console** (CWE-200)
  - **Check:** Check for `console.log()` calls that output tokens, keys, or sensitive data.
  - **Attack:** Users or shoulder surfers see secrets in DevTools console.
  - **Fix:** Remove or guard all console logging of sensitive data. Use a conditional logger that suppresses output in production.

- [ ] `[MEDIUM]` **Secrets in client-side analytics/telemetry** (CWE-200)
  - **Check:** Are URL parameters, form fields, or local storage values sent to analytics platforms (Google Analytics, Mixpanel, etc.)?
  - **Attack:** Third-party analytics services capture and store secrets.
  - **Fix:** Configure analytics to exclude sensitive parameters. Review what data is sent to third parties.

- [ ] `[LOW]` **Missing secret rotation process** (CWE-798)
  - **Check:** How often are secrets rotated? Is there a process? Is it automated?
  - **Attack:** Long-lived secrets increase the window of exposure if compromised.
  - **Fix:** Implement automated secret rotation. Use short-lived tokens where possible (AWS STS, OAuth refresh tokens).

---

## 5. Network and Transport Security

### TLS and HTTPS

- [ ] `[CRITICAL]` **Missing HTTPS in production** (CWE-319)
  - **Check:** Is the production site accessible over plain HTTP? Are API calls made over HTTP?
  - **Attack:** All traffic (including credentials and session tokens) is visible to network attackers.
  - **Fix:** Enforce HTTPS everywhere. Redirect HTTP to HTTPS. Use HSTS.

- [ ] `[HIGH]` **Mixed content** (CWE-319)
  - **Check:** HTTPS page loading resources over HTTP (scripts, stylesheets, images, iframes, XHR/fetch).
  - **Attack:** HTTP resources can be intercepted and modified, injecting malicious content into the HTTPS page.
  - **Fix:** Load all resources over HTTPS. Use protocol-relative URLs (`//`) or absolute HTTPS URLs. Set CSP `upgrade-insecure-requests`.

- [ ] `[HIGH]` **Missing HSTS header** (CWE-523)
  - **Check:** Is `Strict-Transport-Security` header present? Is `max-age` at least 31536000 (1 year)? Is `includeSubDomains` set?
  - **Attack:** First visit over HTTP is vulnerable to MITM (SSL stripping).
  - **Fix:** Set `Strict-Transport-Security: max-age=31536000; includeSubDomains; preload`. Submit to HSTS preload list.

- [ ] `[HIGH]` **Insecure WebSocket connections** (CWE-319)
  - **Check:** WebSocket connections using `ws://` instead of `wss://` in production.
  - **Attack:** WebSocket traffic can be intercepted and modified.
  - **Fix:** Use `wss://` for all WebSocket connections in production. Verify the WebSocket upgrade happens over TLS.

### DNS and Network Attacks

- [ ] `[HIGH]` **DNS rebinding attacks** (CWE-350)
  - **Check:** Does the application validate the `Host` header? Can a malicious domain resolve to localhost/internal IPs?
  - **Attack:** Attacker's domain resolves to `127.0.0.1`, allowing their JavaScript to access localhost services via the browser.
  - **Fix:** Validate `Host` header against an allowlist of expected values. Bind servers to specific interfaces, not `0.0.0.0`. Use authentication on all endpoints.

- [ ] `[MEDIUM]` **Missing certificate pinning (mobile/desktop apps)** (CWE-295)
  - **Check:** For native apps: is the server's TLS certificate or public key pinned? Can a proxy (Burp Suite) MITM the connection?
  - **Attack:** Attacker with a trusted CA certificate (corporate proxy, compromised CA) intercepts all traffic.
  - **Fix:** Implement certificate pinning for native apps. Use backup pins. Plan for rotation.

### Cookie Security

- [ ] `[HIGH]` **Missing Secure flag on cookies** (CWE-614)
  - **Check:** Inspect `Set-Cookie` headers. Are session cookies and auth tokens set with `Secure` flag?
  - **Attack:** Cookie sent over HTTP connection can be intercepted.
  - **Fix:** Set `Secure` flag on all sensitive cookies: `Set-Cookie: session=xxx; Secure`.

- [ ] `[HIGH]` **Missing HttpOnly flag on session cookies** (CWE-1004)
  - **Check:** Can JavaScript access the session cookie? (`document.cookie` includes it?)
  - **Attack:** XSS can steal session cookies.
  - **Fix:** Set `HttpOnly` flag on session cookies: `Set-Cookie: session=xxx; HttpOnly`.

- [ ] `[HIGH]` **Missing SameSite attribute on cookies** (CWE-1275)
  - **Check:** Is `SameSite` attribute set on cookies? `SameSite=Lax` or `SameSite=Strict`?
  - **Attack:** Without SameSite, cookies are sent on cross-site requests (CSRF).
  - **Fix:** Set `SameSite=Lax` at minimum. Use `SameSite=Strict` for highly sensitive cookies.

- [ ] `[MEDIUM]` **Overly broad cookie scope** (CWE-1275)
  - **Check:** Cookie `Domain` set to a parent domain (e.g., `.example.com` when the app is at `app.example.com`).
  - **Attack:** Cookies are sent to all subdomains, including potentially compromised ones.
  - **Fix:** Set the most restrictive `Domain` and `Path` possible for cookies.

---

## 6. Authentication and Session Management

### Session Security

- [ ] `[CRITICAL]` **Session hijacking via token theft** (CWE-384)
  - **Check:** Where are session tokens stored? Are they accessible to JavaScript? Are they transmitted securely?
  - **Attack:** Attacker steals session token via XSS, network interception, or log exposure.
  - **Fix:** Use httpOnly, Secure, SameSite cookies. Use short-lived tokens. Implement token binding. Rotate sessions.

- [ ] `[HIGH]` **Session fixation** (CWE-384)
  - **Check:** Create a session, note the ID, authenticate. Is the session ID the same? Can you set a session ID via URL parameter or cookie?
  - **Attack:** Attacker sets a known session ID before the victim logs in, then uses that session.
  - **Fix:** Generate a new session ID after authentication. Do not accept session IDs from URL parameters.

- [ ] `[HIGH]` **Token storage in localStorage** (CWE-922)
  - **Check:** Are auth tokens stored in localStorage? This makes them accessible to any JavaScript on the page.
  - **Attack:** XSS reads tokens from localStorage, gaining persistent access.
  - **Fix:** Use httpOnly cookies for session management. If tokens must be in JavaScript (SPAs), use short-lived access tokens with refresh via httpOnly cookie.

- [ ] `[MEDIUM]` **Missing session invalidation on logout** (CWE-613)
  - **Check:** After logout, is the session token actually invalidated server-side? Or just deleted client-side?
  - **Attack:** Attacker uses a captured session token even after the user logs out.
  - **Fix:** Invalidate sessions server-side on logout. Maintain a server-side session store.

### JWT Vulnerabilities

- [ ] `[CRITICAL]` **JWT none algorithm attack** (CWE-345)
  - **Check:** Does the server accept JWTs with `"alg": "none"`? Test by modifying a JWT header to `none` and removing the signature.
  - **Attack:** Attacker creates unsigned JWT with arbitrary claims, and the server accepts it.
  - **Fix:** Explicitly allowlist accepted algorithms. Reject `none`. Use a JWT library that requires algorithm specification.

- [ ] `[CRITICAL]` **JWT algorithm confusion (RS256/HS256)** (CWE-345)
  - **Check:** If the server expects RS256, can an attacker send an HS256 JWT signed with the public key?
  - **Attack:** Attacker uses the public RSA key as the HMAC secret, signing arbitrary claims.
  - **Fix:** Explicitly specify the expected algorithm when verifying. Use separate key types for asymmetric and symmetric algorithms.

- [ ] `[HIGH]` **JWT weak secret** (CWE-521)
  - **Check:** If using HS256/HS384/HS512, is the secret strong enough? Can it be brute-forced with tools like `jwt_tool` or `hashcat`?
  - **Attack:** Attacker brute-forces the HMAC secret and can forge arbitrary JWTs.
  - **Fix:** Use a secret with >= 256 bits of entropy. Use a randomly generated key, not a human-readable password.

- [ ] `[HIGH]` **JWT not validated properly** (CWE-347)
  - **Check:** Is the JWT signature verified? Are `exp`, `iss`, `aud` claims checked?
  - **Attack:** Expired, wrong-issuer, or wrong-audience tokens are accepted.
  - **Fix:** Verify signature, expiration, issuer, and audience on every request.

- [ ] `[MEDIUM]` **JWT stored in localStorage** (CWE-922)
  - **Check:** Is the JWT stored in localStorage and sent via Authorization header?
  - **Attack:** XSS can steal the JWT.
  - **Fix:** Use httpOnly cookies to transport JWTs. If you must use Authorization header, keep tokens short-lived (5-15 min) with a refresh token in an httpOnly cookie.

### OAuth Security

- [ ] `[HIGH]` **OAuth state parameter missing** (CWE-352)
  - **Check:** Is the `state` parameter used in OAuth flows? Is it validated on callback?
  - **Attack:** CSRF attack forces user to authenticate with attacker's account (account linking attack).
  - **Fix:** Generate a random `state` parameter, store it in the session, validate on callback.

- [ ] `[HIGH]` **OAuth redirect URI validation** (CWE-601)
  - **Check:** Is the redirect_uri parameter strictly validated? Can it be changed to an attacker-controlled URL?
  - **Attack:** Attacker steals OAuth authorization code or token via open redirect.
  - **Fix:** Register exact redirect URIs. Do not allow wildcards. Validate the full URI, not just the domain.

- [ ] `[MEDIUM]` **OAuth token leakage via Referer** (CWE-200)
  - **Check:** After OAuth callback with token in URL fragment, does the page make requests that leak the token in Referer headers?
  - **Attack:** Token leaks to third-party services via Referer header.
  - **Fix:** Use `Referrer-Policy: no-referrer`. Use authorization code flow (not implicit flow). Clear fragment from URL immediately.

### CSRF Protection

- [ ] `[HIGH]` **Missing CSRF protection** (CWE-352)
  - **Check:** State-changing requests (POST, PUT, DELETE) -- are they protected by CSRF tokens, SameSite cookies, or custom headers?
  - **Attack:** Attacker's page makes authenticated requests on behalf of the logged-in user.
  - **Fix:** Use SameSite=Lax cookies (baseline), CSRF tokens in forms, or custom request headers (e.g., `X-Requested-With`). For APIs, verify `Origin` header.

- [ ] `[HIGH]` **CSRF token not validated server-side** (CWE-352)
  - **Check:** Is the CSRF token present in the request but not actually checked by the server?
  - **Attack:** Presence of a field named `csrf_token` means nothing if the server ignores it.
  - **Fix:** Validate CSRF token on every state-changing request. Use framework-provided CSRF middleware.

- [ ] `[MEDIUM]` **CSRF token reuse across sessions** (CWE-352)
  - **Check:** Does the CSRF token change per session? Is it tied to the user's session?
  - **Attack:** If CSRF tokens are static or predictable, attacker can guess or reuse them.
  - **Fix:** Generate a unique CSRF token per session. Rotate on login.

---

## 7. API Security

### Authorization

- [ ] `[CRITICAL]` **Broken Object Level Authorization (BOLA/IDOR)** (CWE-639)
  - **Check:** For every API endpoint that takes a resource ID, verify the authenticated user is authorized for that specific resource.
  - **Attack:** Attacker changes resource ID to access another user's data.
  - **Fix:** Check `resource.owner === currentUser` on every request. Use authorization middleware. Implement per-resource access control.

- [ ] `[CRITICAL]` **Broken Function Level Authorization** (CWE-285)
  - **Check:** Can a regular user access admin-only API endpoints by simply calling them?
  - **Attack:** Attacker calls admin endpoints (user management, config, etc.) without admin privileges.
  - **Fix:** Implement role-based access control. Check roles server-side on every endpoint. Do not rely on UI hiding.

- [ ] `[HIGH]` **Mass assignment / object injection** (CWE-915)
  - **Check:** Does the API accept arbitrary fields that get written to the database? Can a user set `role: "admin"` or `is_verified: true`?
  - **Attack:** Attacker includes additional fields in a request body that get persisted to the database.
  - **Fix:** Use explicit allowlists for writable fields (DTOs/schemas). Never pass request body directly to ORM/database. Reject unknown fields.

- [ ] `[HIGH]` **Excessive data exposure** (CWE-200)
  - **Check:** Do API responses include more data than the client needs? User objects with password hashes, internal IDs, email addresses, or PII?
  - **Attack:** API returns sensitive fields that the UI doesn't display but the attacker can read.
  - **Fix:** Use response DTOs/serializers. Only return fields the client needs. Different serializers for different roles.

### Input Validation

- [ ] `[HIGH]` **Missing input validation** (CWE-20)
  - **Check:** Are all API parameters validated for type, length, format, and range? Test with: empty strings, very long strings, special characters, negative numbers, arrays where strings expected, nested objects.
  - **Attack:** Unexpected input causes injection, crashes, or logic errors.
  - **Fix:** Validate all input with a schema validation library (Joi, Zod, JSON Schema). Reject invalid input with 400 errors.

- [ ] `[HIGH]` **Missing output encoding** (CWE-116)
  - **Check:** Is API output properly encoded for the context (HTML, JSON, URL, SQL)?
  - **Attack:** Data retrieved from storage is reflected without encoding, enabling injection.
  - **Fix:** Encode output for its context. Use JSON.stringify for JSON APIs. HTML-encode for HTML responses.

- [ ] `[MEDIUM]` **Missing pagination / result limiting** (CWE-770)
  - **Check:** List endpoints that return all results without pagination. Can an attacker request millions of records?
  - **Attack:** Denial of service through resource exhaustion. Data scraping.
  - **Fix:** Implement pagination with reasonable defaults (max 100 items). Enforce maximum page size.

### GraphQL-Specific

- [ ] `[HIGH]` **GraphQL introspection enabled in production** (CWE-200)
  - **Check:** Send `{ __schema { types { name } } }` to the GraphQL endpoint. Does it return the full schema?
  - **Attack:** Attacker maps the entire API surface, including internal types and fields.
  - **Fix:** Disable introspection in production. Enable only in development.

- [ ] `[HIGH]` **GraphQL query depth attack** (CWE-770)
  - **Check:** Can an attacker send deeply nested queries? `{ user { friends { friends { friends { ... } } } } }`
  - **Attack:** Deeply nested queries cause exponential resource usage (denial of service).
  - **Fix:** Implement query depth limiting (max depth 10-15). Use query complexity analysis.

- [ ] `[HIGH]` **GraphQL batching attack** (CWE-770)
  - **Check:** Can multiple queries be sent in a single request? Can this be used to bypass rate limiting?
  - **Attack:** Attacker sends 1000 login mutations in a single batched request.
  - **Fix:** Limit batch size. Apply rate limiting per operation, not per HTTP request.

- [ ] `[MEDIUM]` **GraphQL field suggestions disclosure** (CWE-200)
  - **Check:** Misspell a field name. Does the error suggest valid field names?
  - **Attack:** Attacker enumerates valid fields through suggestions.
  - **Fix:** Disable field suggestions in production. Return generic errors.

### Rate Limiting

- [ ] `[HIGH]` **No rate limiting on API** (CWE-770)
  - **Check:** Can you make hundreds of API requests per second without being throttled?
  - **Attack:** Brute force, data scraping, denial of service, resource exhaustion.
  - **Fix:** Implement rate limiting per IP and per user. Use token bucket or sliding window algorithm. Return `429 Too Many Requests`.

- [ ] `[MEDIUM]` **Rate limiting bypass via headers** (CWE-770)
  - **Check:** Can `X-Forwarded-For` or `X-Real-IP` headers be spoofed to bypass per-IP rate limiting?
  - **Attack:** Attacker sends different IPs via proxy headers to get unlimited requests.
  - **Fix:** Trust proxy headers only from known reverse proxy IPs. Use the last trusted proxy IP.

---

## 8. Frontend-Specific Security

### Security Headers

- [ ] `[HIGH]` **Content-Security-Policy (CSP)** (CWE-693)
  - **Check:** Is CSP set? Does it use `unsafe-inline`, `unsafe-eval`, or wildcard sources for scripts?
  - **Attack:** Without CSP, XSS has no additional barriers. Weak CSP provides false sense of security.
  - **Fix:** `Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; object-src 'none'; base-uri 'self'; frame-ancestors 'self'`.

- [ ] `[HIGH]` **X-Frame-Options / frame-ancestors** (CWE-1021)
  - **Check:** Is `X-Frame-Options: DENY` or `SAMEORIGIN` set? Or CSP `frame-ancestors 'self'`?
  - **Attack:** Clickjacking -- attacker embeds your page in an iframe and tricks users into clicking hidden buttons.
  - **Fix:** Set `X-Frame-Options: DENY` (or SAMEORIGIN) and `frame-ancestors 'self'` in CSP.

- [ ] `[MEDIUM]` **X-Content-Type-Options** (CWE-430)
  - **Check:** Is `X-Content-Type-Options: nosniff` set?
  - **Attack:** Browser MIME-sniffs responses and may execute a file as JavaScript even with a non-JS content type.
  - **Fix:** Set `X-Content-Type-Options: nosniff` on all responses.

- [ ] `[MEDIUM]` **Referrer-Policy** (CWE-200)
  - **Check:** Is `Referrer-Policy` set? What value?
  - **Attack:** Full URL (including tokens, paths) leaked via Referer header to external sites.
  - **Fix:** Set `Referrer-Policy: strict-origin-when-cross-origin` or `no-referrer`.

- [ ] `[MEDIUM]` **Subresource Integrity (SRI)** (CWE-353)
  - **Check:** External scripts and stylesheets -- do they have `integrity` attributes?
  - **Attack:** Compromised CDN serves malicious code.
  - **Fix:** Add `integrity="sha384-..."` to all external `<script>` and `<link>` tags.

- [ ] `[LOW]` **Permissions-Policy** (CWE-693)
  - **Check:** Is `Permissions-Policy` header set to restrict browser features?
  - **Attack:** Third-party scripts access camera, microphone, or geolocation.
  - **Fix:** `Permissions-Policy: camera=(), microphone=(), geolocation=(), payment=()`.

### Client-Side Attacks

- [ ] `[HIGH]` **Open redirects** (CWE-601)
  - **Check:** Parameters like `?redirect=`, `?next=`, `?returnUrl=`, `?url=`. Can they redirect to external domains?
  - **Attack:** Phishing -- attacker creates link `https://yourapp.com/login?next=https://evil.com` that redirects after login.
  - **Fix:** Validate redirect URLs. Only allow relative paths or allowlisted domains. Never redirect to user-supplied absolute URLs.

- [ ] `[HIGH]` **Clickjacking** (CWE-1021)
  - **Check:** Can your application be embedded in an iframe on another domain? (See X-Frame-Options above.)
  - **Attack:** Attacker overlays invisible iframe on a decoy page, tricking user into clicking actions in your app.
  - **Fix:** Set `X-Frame-Options: DENY` and CSP `frame-ancestors 'none'` (or `'self'` if framing is needed).

- [ ] `[HIGH]` **postMessage origin validation** (CWE-346)
  - **Check:** All `window.addEventListener('message', ...)` handlers. Is `event.origin` checked?
  - **Attack:** Any window can send postMessage. Without origin check, attacker injects data or triggers actions.
  - **Fix:** Always validate `event.origin` against a strict allowlist before processing the message.

- [ ] `[HIGH]` **Third-party script risks** (CWE-829)
  - **Check:** List all third-party scripts (analytics, chat widgets, ads, social buttons). Are they loaded with SRI? What permissions do they have?
  - **Attack:** Compromised or malicious third-party script has full access to the page (DOM, cookies, localStorage).
  - **Fix:** Minimize third-party scripts. Use SRI. Load in sandboxed iframes where possible. Use CSP to restrict script sources. Audit regularly.

- [ ] `[MEDIUM]` **Client-side data exposure in DevTools** (CWE-200)
  - **Check:** Open DevTools on production. Check Network tab for sensitive data in responses. Check Application tab for sensitive localStorage/cookie values. Check Console for logged secrets.
  - **Attack:** Users or attackers inspect client-side data stores.
  - **Fix:** Minimize sensitive data sent to the client. Encrypt sensitive localStorage values. Suppress console output in production.

- [ ] `[MEDIUM]` **Autocomplete on sensitive fields** (CWE-200)
  - **Check:** Password fields, credit card fields, SSN fields -- is `autocomplete="off"` set where appropriate?
  - **Attack:** Browser stores and suggests sensitive values on shared computers.
  - **Fix:** Use `autocomplete="off"` or `autocomplete="new-password"` on sensitive fields.

---

## 9. Infrastructure and Deployment

### Container Security

- [ ] `[HIGH]` **Container running as root** (CWE-250)
  - **Check:** Does the Dockerfile use `USER` directive? Run `docker exec <container> whoami`.
  - **Attack:** Container escape vulnerabilities have greater impact when running as root.
  - **Fix:** Add `USER nonroot` to Dockerfile. Use distroless or Alpine base images. Set `runAsNonRoot: true` in Kubernetes.

- [ ] `[HIGH]` **Excessive container capabilities** (CWE-250)
  - **Check:** Is the container running with `--privileged`? Are unnecessary Linux capabilities enabled?
  - **Attack:** Privileged containers can escape to the host.
  - **Fix:** Drop all capabilities and add only required ones: `--cap-drop ALL --cap-add NET_BIND_SERVICE`.

- [ ] `[HIGH]` **Secrets in Docker environment variables** (CWE-526)
  - **Check:** `docker inspect <container>` shows all environment variables, including secrets.
  - **Attack:** Anyone with Docker access reads all secrets.
  - **Fix:** Use Docker secrets, Kubernetes secrets (encrypted at rest), or external secrets managers. Avoid environment variables for highly sensitive secrets.

- [ ] `[MEDIUM]` **Exposed container ports** (CWE-284)
  - **Check:** `docker ps` -- are unnecessary ports published to the host?
  - **Attack:** Exposed ports increase attack surface.
  - **Fix:** Only publish ports that need external access. Use internal Docker networks for inter-container communication.

- [ ] `[MEDIUM]` **Base image vulnerabilities** (CWE-1035)
  - **Check:** Scan base images with `trivy image <image>` or `docker scout`.
  - **Attack:** Vulnerable base image libraries are exploitable inside the container.
  - **Fix:** Use minimal base images (distroless, Alpine). Scan regularly. Update base images.

### Cloud and Kubernetes

- [ ] `[CRITICAL]` **Over-provisioned IAM roles** (CWE-250)
  - **Check:** Review IAM policies for `*` permissions, `Action: "*"`, or `Resource: "*"`.
  - **Attack:** Compromised service/user with excessive permissions can access or modify all resources.
  - **Fix:** Follow least privilege. Grant only needed permissions. Use separate roles per service. Audit with IAM Access Analyzer.

- [ ] `[HIGH]` **Kubernetes secrets not encrypted at rest** (CWE-312)
  - **Check:** Are Kubernetes secrets encrypted at rest (etcd encryption)? By default, they are base64-encoded (NOT encrypted).
  - **Attack:** Anyone with etcd access reads all secrets.
  - **Fix:** Enable encryption at rest for etcd. Use external secrets operators (AWS Secrets Manager, Vault).

- [ ] `[HIGH]` **Exposed admin panels** (CWE-200)
  - **Check:** Admin dashboards, Kubernetes Dashboard, phpMyAdmin, Kibana, Grafana -- are they publicly accessible?
  - **Attack:** Attacker accesses admin panels directly.
  - **Fix:** Restrict admin panels to VPN/internal network. Add authentication. Use IP allowlists.

- [ ] `[MEDIUM]` **Debug endpoints in production** (CWE-489)
  - **Check:** `/debug`, `/metrics`, `/healthz`, `/pprof`, `/actuator`, `/_debug`, `/api/debug`. Are they authenticated?
  - **Attack:** Debug endpoints expose internal state, performance data, or allow config changes.
  - **Fix:** Remove or authenticate all debug endpoints in production. Use separate debug ports not exposed externally.

### Web Server Configuration

- [ ] `[MEDIUM]` **Directory listing enabled** (CWE-548)
  - **Check:** Try accessing a directory URL (e.g., `/static/`, `/uploads/`). Does it show a file listing?
  - **Attack:** Attacker discovers hidden files, backups, configuration files.
  - **Fix:** Disable directory listing in web server config. Nginx: `autoindex off;`. Apache: `Options -Indexes`.

- [ ] `[MEDIUM]` **Server version disclosure** (CWE-200)
  - **Check:** Check response headers for `Server: nginx/1.25.3`, `X-Powered-By: Express`, etc.
  - **Attack:** Version info helps attacker find known CVEs.
  - **Fix:** Remove version from `Server` header. Nginx: `server_tokens off;`. Express: `app.disable('x-powered-by')`.

- [ ] `[MEDIUM]` **Error page information disclosure** (CWE-209)
  - **Check:** Trigger 404, 500 errors. Do error pages reveal framework, version, file paths, or stack traces?
  - **Attack:** Error pages reveal internal architecture details.
  - **Fix:** Use custom error pages that reveal nothing about the technology stack.

- [ ] `[LOW]` **Missing HTTP to HTTPS redirect** (CWE-319)
  - **Check:** Access the site via HTTP. Is there an automatic redirect to HTTPS?
  - **Attack:** Users accessing via HTTP have traffic intercepted.
  - **Fix:** Configure server to redirect all HTTP to HTTPS with 301 redirect.

- [ ] `[LOW]` **Exposed backup files** (CWE-530)
  - **Check:** Try accessing common backup paths: `/backup.sql`, `/db.sql.gz`, `/.env.bak`, `/config.php.bak`, `/index.php~`.
  - **Attack:** Backup files contain source code or database dumps.
  - **Fix:** Block access to backup file extensions in web server config. Do not store backups in the webroot.

---

## 10. Low-Hanging Fruit Checklist

> Quick-scan section for the most commonly missed items. Start here for a fast pass.

### Git and Repository Hygiene

- [ ] `[CRITICAL]` **Is there a .gitignore that excludes .env, credentials, and private keys?**
  - **Check:** `.gitignore` must include: `.env`, `.env.*`, `*.pem`, `*.key`, `credentials.json`, `serviceAccountKey.json`, `*.p12`.
  - **Fix:** Update `.gitignore`. Run `git rm --cached` for any tracked files.

- [ ] `[CRITICAL]` **Are all API keys server-side only?**
  - **Check:** Grep frontend bundle for API key patterns. Check `process.env` usage in frontend code.
  - **Fix:** Move all secret keys to server-side environment. Proxy API calls through your backend.

### Production Configuration

- [ ] `[CRITICAL]` **Is debug mode disabled in production?**
  - **Check:** Verify `NODE_ENV=production`, `DEBUG=false`, framework-specific debug flags.
  - **Fix:** Set production environment variables in deployment config.

- [ ] `[HIGH]` **Are error messages generic (no stack traces)?**
  - **Check:** Trigger errors. Verify no stack traces, file paths, or SQL queries in responses.
  - **Fix:** Implement global error handler that returns generic messages.

- [ ] `[HIGH]` **Is HTTPS enforced?**
  - **Check:** Try accessing via HTTP. Verify redirect and HSTS.
  - **Fix:** Configure HTTPS redirect and HSTS header.

- [ ] `[HIGH]` **Are dependencies up to date?**
  - **Check:** Run `npm audit`, `cargo audit`, or equivalent. Check for high/critical vulnerabilities.
  - **Fix:** Update dependencies. Set up automated scanning.

### Access Control

- [ ] `[HIGH]` **Are admin routes protected?**
  - **Check:** Access admin endpoints without authentication/authorization.
  - **Fix:** Add auth middleware to all admin routes.

- [ ] `[HIGH]` **Is user input sanitized at system boundaries?**
  - **Check:** Test all input fields with XSS payloads, SQL injection, and command injection.
  - **Fix:** Validate and sanitize at every system boundary (API, database, rendering).

### Cryptography

- [ ] `[CRITICAL]` **Are passwords hashed with bcrypt/argon2 (not MD5/SHA1)?**
  - **Check:** Review password storage code. Grep for md5, sha1, sha256 used on passwords.
  - **Fix:** Migrate to bcrypt (cost >= 12) or argon2id.

- [ ] `[HIGH]` **Is there rate limiting on login/signup?**
  - **Check:** Attempt rapid login attempts. Verify rate limiting kicks in.
  - **Fix:** Implement rate limiting: 5 attempts per minute per IP, 10 per account.

### Data Protection

- [ ] `[HIGH]` **Are sensitive cookies HttpOnly + Secure + SameSite?**
  - **Check:** Inspect Set-Cookie headers for session cookies.
  - **Fix:** Add all three flags to session cookies.

- [ ] `[MEDIUM]` **Is there a Content-Security-Policy header?**
  - **Check:** Inspect response headers.
  - **Fix:** Add a strict CSP header.

- [ ] `[MEDIUM]` **Are file uploads validated (type, size, content)?**
  - **Check:** Upload unexpected file types. Check for size limits.
  - **Fix:** Validate MIME type, extension, file size, and content server-side.

- [ ] `[LOW]` **Is there a security.txt file?**
  - **Check:** Try accessing `/.well-known/security.txt`.
  - **Fix:** Create `/.well-known/security.txt` with security contact and disclosure policy.

---

## 11. KMD-Specific Findings

> This section documents the security audit findings for our KMD (K.md) web application -- a local development dashboard built with Rust (axum) + TypeScript.

### Architecture Context

KMD is a localhost web server that provides a development dashboard for managing scripts, documentation, ports, terminal access, and environment variables. It runs on `127.0.0.1` with a web UI accessible via the browser.

**Tech Stack:** Rust (axum, tokio, rusqlite), TypeScript (esbuild), ammonia HTML sanitizer.

### What We Found (Audit Findings)

#### Critical / High Severity

- [ ] `[CRITICAL]` **Shell exec endpoint unprotected** (CWE-78)
  - **Finding:** The `/api/shell/exec` endpoint allowed arbitrary command execution without authentication. Any page open in the browser (or malicious script) could execute shell commands.
  - **Status:** FIXED -- Bearer token auth required, token generated at startup.

- [ ] `[CRITICAL]` **Terminal WebSocket unprotected** (CWE-284)
  - **Finding:** The `/ws/terminal` WebSocket endpoint provided full shell access without any authentication.
  - **Status:** FIXED -- Auth token required as query parameter, validated against startup-generated token.

- [ ] `[HIGH]` **DNS rebinding attack surface** (CWE-350)
  - **Finding:** A malicious website could make requests to `localhost:4444` and interact with the KMD API if the Host header was not validated.
  - **Status:** FIXED -- `validate_host` middleware blocks requests with non-localhost Host headers.

- [ ] `[HIGH]` **Missing CSRF protection on mutating endpoints** (CWE-352)
  - **Finding:** POST/PUT/DELETE/PATCH endpoints had no CSRF protection. A malicious page could make cross-origin requests using form submissions.
  - **Status:** FIXED -- Custom `X-KMD-Client` header required on all mutating requests (not sent by browsers on cross-origin form submissions). Origin header validated.

- [ ] `[HIGH]` **Environment variable values exposed in compare API** (CWE-200)
  - **Finding:** The `/api/env/compare` endpoint returned full plaintext values of environment variables, potentially exposing secrets.
  - **Status:** FIXED -- Compare uses hashed values; plaintext secrets are never returned.

- [ ] `[HIGH]` **No rate limiting on API endpoints** (CWE-770)
  - **Finding:** All API endpoints could be called at unlimited rate, enabling abuse and resource exhaustion.
  - **Status:** FIXED -- Token bucket rate limiter applied to all mutating endpoints (burst of 20, refill 5/sec).

#### Medium Severity

- [ ] `[MEDIUM]` **Mermaid script loaded without SRI** (CWE-353)
  - **Finding:** Vendored `mermaid.min.js` loaded without Subresource Integrity hash. If the file were tampered with, there would be no detection.
  - **Status:** PLANNED -- SRI hash to be computed and added to script tag.

- [ ] `[MEDIUM]` **data: URI scheme allowed broadly** (CWE-79)
  - **Finding:** Ammonia HTML sanitizer allowed `data:` URIs in all tag attributes, enabling potential XSS via `data:text/html` in href attributes.
  - **Status:** PLANNED -- Restrict `data:` URIs to `data:image/*` on `<img src>` only.

- [ ] `[MEDIUM]` **Console logging in production** (CWE-200)
  - **Finding:** Debug `console.log` statements active in production frontend, potentially exposing internal state.
  - **Status:** PLANNED -- Conditional logger that suppresses output when not on localhost.

- [ ] `[MEDIUM]` **Script name not validated against package.json** (CWE-20)
  - **Finding:** The script execution endpoint accepted arbitrary script names without verifying they exist in `package.json`.
  - **Status:** PLANNED -- Validate script name exists in package.json before spawning.

- [ ] `[MEDIUM]` **Lockfile lacks integrity nonce** (CWE-345)
  - **Finding:** The lockfile (used to detect running KMD instances) had no integrity verification, allowing potential impersonation.
  - **Status:** PLANNED -- Nonce to be added to lockfile with health check verification.

#### Defense-in-Depth Measures Already Present

- [ ] `[INFO]` **Server-side HTML sanitization (ammonia)**
  - **Finding:** All markdown-rendered HTML passes through ammonia sanitizer on the server. This prevents stored XSS from markdown content.
  - **Status:** IN PLACE -- ammonia configured with safe tag/attribute allowlists.

- [ ] `[INFO]` **Client-side HTML sanitization (security.ts)**
  - **Finding:** Defense-in-depth client-side sanitizer implemented in `client/lib/security.ts`. Strips `<script>`, `<iframe>`, `<object>`, `<embed>`, `<form>`, `<base>`, `<link>`, `<meta>` tags. Removes event handler attributes and `javascript:`/`vbscript:` URLs.
  - **Status:** IN PLACE.

- [ ] `[INFO]` **Path traversal protection**
  - **Finding:** Client-side path validation rejects `..`, absolute paths, and null bytes. Server-side uses `canonicalize()` + `starts_with()` to prevent directory traversal.
  - **Status:** IN PLACE -- both client and server validate paths.

- [ ] `[INFO]` **Security headers**
  - **Finding:** `add_security_headers` middleware adds CSP, X-Content-Type-Options, X-Frame-Options, and other security headers to all responses.
  - **Status:** IN PLACE.

- [ ] `[INFO]` **Localhost binding**
  - **Finding:** KMD binds to `127.0.0.1` only, not `0.0.0.0`. This prevents direct remote access.
  - **Status:** IN PLACE.

- [ ] `[INFO]` **kmdFetch CSRF wrapper**
  - **Finding:** All frontend API calls use `kmdFetch()` from `client/lib/security.ts` which automatically adds the `X-KMD-Client` header to mutating requests.
  - **Status:** IN PLACE.

### What Remains as Accepted Risk

- [ ] `[ACCEPTED]` **Local network trust model**
  - **Justification:** KMD is a local development tool running on localhost. The threat model assumes the local machine is trusted. Remote attacks are mitigated by localhost binding + Host header validation + CSRF protection, but a fully compromised local machine could bypass all protections. This is accepted because a compromised local machine already has shell access.

- [ ] `[ACCEPTED]` **No authentication for read-only endpoints**
  - **Justification:** Read-only GET endpoints (docs, scripts list, ports) do not require authentication. DNS rebinding and CSRF protections (Host header validation, Origin checking) provide the access control boundary. Full auth was deemed unnecessary friction for a local dev tool.

- [ ] `[ACCEPTED]` **Vendored mermaid.min.js not audited line-by-line**
  - **Justification:** Mermaid is a widely-used open-source library. Full audit of the minified 2MB+ file is impractical. Risk is mitigated by CSP, client-side sanitization, and server-side ammonia sanitization. SRI will be added for tamper detection.

- [ ] `[ACCEPTED]` **Browser extension attack surface**
  - **Justification:** Browser extensions with host permissions can read/modify any page content, including KMD. This is a browser-level trust boundary that cannot be mitigated by web application security measures.

---

## Appendix A: Tools and Commands

Quick reference for running security checks:

```bash
# Dependency vulnerability scanning
npm audit                          # Node.js
cargo audit                        # Rust
pip audit                          # Python
snyk test                          # Multi-language

# Secret scanning
trufflehog git file://. --only-verified
gitleaks detect --source .
git-secrets --scan

# SAST (Static Application Security Testing)
semgrep --config=auto .
bandit -r .                        # Python
cargo clippy -- -D warnings        # Rust

# Container scanning
trivy image <image-name>
docker scout cves <image-name>

# TLS testing
nmap --script ssl-enum-ciphers -p 443 <host>
testssl.sh <host>

# Header checking
curl -I https://<host> | grep -iE "content-security|x-frame|x-content|strict-transport|referrer-policy|permissions-policy"

# SRI hash generation
openssl dgst -sha384 -binary <file> | openssl base64 -A | sed 's/^/sha384-/'
```

## Appendix B: Severity Rating Criteria

| Severity | Description | CVSS Range |
|----------|-------------|------------|
| **CRITICAL** | Remotely exploitable with high impact. No authentication required or leads to full system compromise. | 9.0 - 10.0 |
| **HIGH** | Significant risk. Requires some preconditions but leads to data breach, privilege escalation, or significant functionality abuse. | 7.0 - 8.9 |
| **MEDIUM** | Moderate risk. Requires specific conditions or chaining with other vulnerabilities. Limited impact. | 4.0 - 6.9 |
| **LOW** | Defense-in-depth. Best practice recommendation. Minimal direct impact. | 0.1 - 3.9 |
| **INFO** | Informational. Already implemented security measure documented for completeness. | N/A |

## Appendix C: References

- [OWASP Top 10 (2021)](https://owasp.org/Top10/)
- [OWASP Application Security Verification Standard (ASVS)](https://owasp.org/www-project-application-security-verification-standard/)
- [CWE/SANS Top 25](https://cwe.mitre.org/top25/archive/2023/2023_top25_list.html)
- [OWASP Cheat Sheet Series](https://cheatsheetseries.owasp.org/)
- [Mozilla Web Security Guidelines](https://infosec.mozilla.org/guidelines/web_security)
- [OWASP API Security Top 10](https://owasp.org/API-Security/)
- [Content Security Policy Reference](https://content-security-policy.com/)
- [JWT Best Practices (RFC 8725)](https://www.rfc-editor.org/rfc/rfc8725)
