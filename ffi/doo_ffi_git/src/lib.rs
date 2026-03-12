//! # Doo FFI Git
//!
//! Native git operations for Doo via libgit2 (git2-rs).
//! Zero subprocess overhead — all operations are in-process.
//!
//! ## Functions
//! - `doo_git_init(path)` — Initialize a new git repository
//! - `doo_git_clone(url, path)` — Clone a remote repository
//! - `doo_git_commit_all(path, msg, name, email)` — Add all + commit + return short hash
//! - `doo_git_push(path, remote, branch)` — Push to remote
//! - `doo_git_pull(path)` — Fetch + fast-forward merge from origin
//! - `doo_git_is_dirty(path)` — Check if working dir has changes
//! - `doo_git_stash(path)` — Stash uncommitted changes
//! - `doo_git_stash_pop(path)` — Pop last stash
//! - `doo_git_has_remote(path)` — Check if any remote is configured
//! - `doo_git_head_short(path)` — Get short HEAD commit hash
//!
//! ## Design
//! - Pure standalone FFI package (not coupled to compiler)
//! - Every function uses catch_unwind for safety
//! - DooResult return type: tag=0 Ok, tag=1 Err (RFC 7807)
//! - All string params are C strings (*const c_char)

use std::os::raw::c_char;
use std::panic;

use doo_ffi_core::helpers::{c_to_string_lossy, make_ok_string, make_ok_void};
use doo_ffi_core::result::DooResult;

use git2::{Cred, PushOptions, RemoteCallbacks, Repository, Signature, StatusOptions};

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

// ============================================================================
// Helpers
// ============================================================================

fn make_err(msg: &str) -> *mut DooResult {
    doo_ffi_core::helpers::make_err_rfc7807(500, msg)
}

fn make_panic_err() -> *mut DooResult {
    make_err("Git FFI: internal panic")
}

fn bool_to_i8(b: bool) -> i8 {
    if b {
        1
    } else {
        0
    }
}

// ============================================================================
// doo_git_init — Initialize a new git repository
// ============================================================================

/// Initialize a git repository at the given path.
/// Creates the directory if needed (git2 handles this).
#[no_mangle]
pub extern "C" fn doo_git_init(path: *const c_char) -> *mut DooResult {
    let result = panic::catch_unwind(|| {
        let path_str = c_to_string_lossy(path);
        match Repository::init(&path_str) {
            Ok(_) => make_ok_void(),
            Err(e) => make_err(&format!("git init failed: {}", e)),
        }
    });
    result.unwrap_or_else(|_| make_panic_err())
}

// ============================================================================
// doo_git_init_bare — Initialize a bare git repository
// ============================================================================

/// Initialize a bare git repository at the given path.
/// Bare repos have no working directory — used as remotes for push/pull.
#[no_mangle]
pub extern "C" fn doo_git_init_bare(path: *const c_char) -> *mut DooResult {
    let result = panic::catch_unwind(|| {
        let path_str = c_to_string_lossy(path);
        match Repository::init_bare(&path_str) {
            Ok(_) => make_ok_void(),
            Err(e) => make_err(&format!("git init --bare failed: {}", e)),
        }
    });
    result.unwrap_or_else(|_| make_panic_err())
}

// ============================================================================
// doo_git_clone — Clone a remote repository
// ============================================================================

/// Clone a remote repository from `url` into `path`.
#[no_mangle]
pub extern "C" fn doo_git_clone(url: *const c_char, path: *const c_char) -> *mut DooResult {
    let result = panic::catch_unwind(|| {
        let url_str = c_to_string_lossy(url);
        let path_str = c_to_string_lossy(path);
        match Repository::clone(&url_str, &path_str) {
            Ok(_) => make_ok_void(),
            Err(e) => make_err(&format!("git clone failed: {}", e)),
        }
    });
    result.unwrap_or_else(|_| make_panic_err())
}

// ============================================================================
// doo_git_commit_all — Add all + commit + return short hash
// ============================================================================

/// Stage all changes (git add -A), commit with the given message and author,
/// and return the short commit hash (7 chars).
///
/// This is the primary high-level function for the DooCloud engine:
/// one native call replaces 3+ subprocess calls.
#[no_mangle]
pub extern "C" fn doo_git_commit_all(
    path: *const c_char,
    message: *const c_char,
    author_name: *const c_char,
    author_email: *const c_char,
) -> *mut DooResult {
    let result = panic::catch_unwind(|| {
        let path_str = c_to_string_lossy(path);
        let msg_str = c_to_string_lossy(message);
        let name_str = c_to_string_lossy(author_name);
        let email_str = c_to_string_lossy(author_email);

        // Open repository
        let repo = match Repository::open(&path_str) {
            Ok(r) => r,
            Err(e) => return make_err(&format!("git open failed: {}", e)),
        };

        // Stage all changes (equivalent to `git add -A`)
        let mut index = match repo.index() {
            Ok(i) => i,
            Err(e) => return make_err(&format!("git index failed: {}", e)),
        };
        if let Err(e) = index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None) {
            return make_err(&format!("git add failed: {}", e));
        }
        // Also remove deleted files from index
        if let Err(e) = index.update_all(["*"].iter(), None) {
            return make_err(&format!("git update index failed: {}", e));
        }
        if let Err(e) = index.write() {
            return make_err(&format!("git index write failed: {}", e));
        }

        // Write index as tree
        let tree_oid = match index.write_tree() {
            Ok(oid) => oid,
            Err(e) => return make_err(&format!("git write tree failed: {}", e)),
        };
        let tree = match repo.find_tree(tree_oid) {
            Ok(t) => t,
            Err(e) => return make_err(&format!("git find tree failed: {}", e)),
        };

        // Create signature
        let sig = match Signature::now(&name_str, &email_str) {
            Ok(s) => s,
            Err(e) => return make_err(&format!("git signature failed: {}", e)),
        };

        // Get parent commit (if any — first commit has no parents)
        let parent_commit = repo.head().ok().and_then(|head| head.peel_to_commit().ok());

        let parents: Vec<&git2::Commit> = parent_commit.iter().collect();

        // Create the commit
        let commit_oid = match repo.commit(Some("HEAD"), &sig, &sig, &msg_str, &tree, &parents) {
            Ok(oid) => oid,
            Err(e) => return make_err(&format!("git commit failed: {}", e)),
        };

        // Return short hash (first 7 chars)
        let short_hash = &commit_oid.to_string()[..7];
        make_ok_string(short_hash)
    });
    result.unwrap_or_else(|_| make_panic_err())
}

// ============================================================================
// doo_git_push — Push to remote
// ============================================================================

/// Push the current branch to the specified remote and branch.
/// Uses default credentials from the system.
#[no_mangle]
pub extern "C" fn doo_git_push(
    path: *const c_char,
    remote_name: *const c_char,
    branch: *const c_char,
) -> *mut DooResult {
    let result = panic::catch_unwind(|| {
        let path_str = c_to_string_lossy(path);
        let remote_str = c_to_string_lossy(remote_name);
        let branch_str = c_to_string_lossy(branch);

        let repo = match Repository::open(&path_str) {
            Ok(r) => r,
            Err(e) => return make_err(&format!("git open failed: {}", e)),
        };

        let mut remote = match repo.find_remote(&remote_str) {
            Ok(r) => r,
            Err(e) => return make_err(&format!("git find remote '{}' failed: {}", remote_str, e)),
        };

        let refspec = format!("refs/heads/{}:refs/heads/{}", branch_str, branch_str);
        match remote.push(&[&refspec], None) {
            Ok(_) => make_ok_void(),
            Err(e) => make_err(&format!("git push failed: {}", e)),
        }
    });
    result.unwrap_or_else(|_| make_panic_err())
}

// ============================================================================
// doo_git_force_push — Force push to remote (overwrites remote history)
// ============================================================================

/// Force push the current branch to the specified remote.
/// Uses `+` refspec prefix to force-update the remote branch.
/// Needed when local and remote histories diverge (e.g., fresh init vs remote README).
#[no_mangle]
pub extern "C" fn doo_git_force_push(
    path: *const c_char,
    remote_name: *const c_char,
    branch: *const c_char,
) -> *mut DooResult {
    let result = panic::catch_unwind(|| {
        let path_str = c_to_string_lossy(path);
        let remote_str = c_to_string_lossy(remote_name);
        let branch_str = c_to_string_lossy(branch);

        let repo = match Repository::open(&path_str) {
            Ok(r) => r,
            Err(e) => return make_err(&format!("git open failed: {}", e)),
        };

        let mut remote = match repo.find_remote(&remote_str) {
            Ok(r) => r,
            Err(e) => return make_err(&format!("git find remote '{}' failed: {}", remote_str, e)),
        };

        // '+' prefix = force push (like git push --force)
        let refspec = format!("+refs/heads/{}:refs/heads/{}", branch_str, branch_str);
        match remote.push(&[&refspec], None) {
            Ok(_) => make_ok_void(),
            Err(e) => make_err(&format!("git force push failed: {}", e)),
        }
    });
    result.unwrap_or_else(|_| make_panic_err())
}

// ============================================================================
// doo_git_push_with_token — Push to remote with token authentication
// ============================================================================

/// Push the current branch using a token for HTTPS authentication.
/// For GitHub: username = "x-access-token", token = OAuth/PAT token.
/// For other providers: username/token as appropriate.
#[no_mangle]
pub extern "C" fn doo_git_push_with_token(
    path: *const c_char,
    remote_name: *const c_char,
    branch: *const c_char,
    username: *const c_char,
    token: *const c_char,
) -> *mut DooResult {
    let result = panic::catch_unwind(|| {
        let path_str = c_to_string_lossy(path);
        let remote_str = c_to_string_lossy(remote_name);
        let branch_str = c_to_string_lossy(branch);
        let username_str = c_to_string_lossy(username);
        let token_str = c_to_string_lossy(token);

        let repo = match Repository::open(&path_str) {
            Ok(r) => r,
            Err(e) => return make_err(&format!("git open failed: {}", e)),
        };

        let mut remote = match repo.find_remote(&remote_str) {
            Ok(r) => r,
            Err(e) => return make_err(&format!("git find remote '{}' failed: {}", remote_str, e)),
        };

        // Set up credential callback for token-based auth
        let mut callbacks = RemoteCallbacks::new();
        callbacks.credentials(move |_url, _username_from_url, _allowed_types| {
            Cred::userpass_plaintext(&username_str, &token_str)
        });

        let mut push_options = PushOptions::new();
        push_options.remote_callbacks(callbacks);

        let refspec = format!("refs/heads/{}:refs/heads/{}", branch_str, branch_str);
        let res = match remote.push(&[&refspec], Some(&mut push_options)) {
            Ok(_) => make_ok_void(),
            Err(e) => make_err(&format!("git push failed: {}", e)),
        };
        res
    });
    result.unwrap_or_else(|_| make_panic_err())
}

// ============================================================================
// doo_git_pull — Fetch + fast-forward merge from origin
// ============================================================================

/// Fetch from origin and fast-forward merge the current branch.
/// Equivalent to `git pull --ff-only` behavior.
#[no_mangle]
pub extern "C" fn doo_git_pull(path: *const c_char) -> *mut DooResult {
    let result = panic::catch_unwind(|| {
        let path_str = c_to_string_lossy(path);

        let repo = match Repository::open(&path_str) {
            Ok(r) => r,
            Err(e) => return make_err(&format!("git open failed: {}", e)),
        };

        // Find the "origin" remote
        let mut remote = match repo.find_remote("origin") {
            Ok(r) => r,
            Err(e) => return make_err(&format!("git find remote 'origin' failed: {}", e)),
        };

        // Fetch
        if let Err(e) = remote.fetch(&[] as &[&str], None, None) {
            return make_err(&format!("git fetch failed: {}", e));
        }

        // Get the current branch name
        let head = match repo.head() {
            Ok(h) => h,
            Err(e) => return make_err(&format!("git head failed: {}", e)),
        };

        let branch_name = match head.shorthand() {
            Some(name) => name.to_string(),
            None => return make_err("git pull: could not determine branch name"),
        };

        // Find the fetch head (FETCH_HEAD or origin/<branch>)
        let fetch_head_ref = format!("refs/remotes/origin/{}", branch_name);
        let fetch_head = match repo.find_reference(&fetch_head_ref) {
            Ok(r) => r,
            Err(e) => return make_err(&format!("git pull: no remote tracking branch: {}", e)),
        };

        let fetch_commit = match repo.reference_to_annotated_commit(&fetch_head) {
            Ok(c) => c,
            Err(e) => return make_err(&format!("git pull: annotated commit failed: {}", e)),
        };

        // Analyze merge
        let (analysis, _) = match repo.merge_analysis(&[&fetch_commit]) {
            Ok(a) => a,
            Err(e) => return make_err(&format!("git merge analysis failed: {}", e)),
        };

        if analysis.is_up_to_date() {
            return make_ok_void();
        }

        if analysis.is_fast_forward() {
            // Fast-forward: just move HEAD
            let target_oid = fetch_commit.id();
            let mut reference = match repo.find_reference("HEAD") {
                Ok(r) => r,
                Err(e) => return make_err(&format!("git pull ff: HEAD ref failed: {}", e)),
            };
            if let Err(e) = reference.set_target(target_oid, "doo pull: fast-forward") {
                // Try direct head set instead
                if let Err(e2) = repo.set_head_detached(target_oid) {
                    return make_err(&format!("git pull ff failed: {}, {}", e, e2));
                }
            }
            // Checkout to update working directory
            if let Err(e) = repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force())) {
                return make_err(&format!("git pull checkout failed: {}", e));
            }
            return make_ok_void();
        }

        // Non-fast-forward: we don't handle complex merges automatically
        make_err("git pull: not a fast-forward merge (manual resolution needed)")
    });
    result.unwrap_or_else(|_| make_panic_err())
}

// ============================================================================
// doo_git_is_dirty — Check if working directory has uncommitted changes
// ============================================================================

/// Returns 1 (true) if the working directory has uncommitted changes, 0 (false) otherwise.
/// Returns i32 for C ABI compatibility (matches Doo Bool → i32 at FFI boundary).
#[no_mangle]
pub extern "C" fn doo_git_is_dirty(path: *const c_char) -> i32 {
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let path_str = c_to_string_lossy(path);

        let repo = match Repository::open(&path_str) {
            Ok(r) => r,
            Err(_) => return 0i32, // Can't open → not dirty
        };

        let mut opts = StatusOptions::new();
        opts.include_untracked(true);
        opts.recurse_untracked_dirs(true);

        let is_dirty = repo
            .statuses(Some(&mut opts))
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        if is_dirty {
            1i32
        } else {
            0i32
        }
    }));
    result.unwrap_or(0i32)
}

// ============================================================================
// doo_git_stash — Stash uncommitted changes
// ============================================================================

/// Stash all uncommitted changes in the working directory.
/// Uses a default stash message and includes untracked files.
#[no_mangle]
pub extern "C" fn doo_git_stash(path: *const c_char) -> *mut DooResult {
    let result = panic::catch_unwind(|| {
        let path_str = c_to_string_lossy(path);

        let mut repo = match Repository::open(&path_str) {
            Ok(r) => r,
            Err(e) => return make_err(&format!("git open failed: {}", e)),
        };

        let sig = match Signature::now("DooCloud Engine", "engine@doocloud.dev") {
            Ok(s) => s,
            Err(e) => return make_err(&format!("git signature failed: {}", e)),
        };

        match repo.stash_save(
            &sig,
            "DooCloud auto-stash",
            Some(git2::StashFlags::INCLUDE_UNTRACKED),
        ) {
            Ok(_oid) => make_ok_void(),
            Err(e) => make_err(&format!("git stash failed: {}", e)),
        }
    });
    result.unwrap_or_else(|_| make_panic_err())
}

// ============================================================================
// doo_git_stash_pop — Pop the last stash
// ============================================================================

/// Pop the most recent stash entry (index 0), restoring changes.
#[no_mangle]
pub extern "C" fn doo_git_stash_pop(path: *const c_char) -> *mut DooResult {
    let result = panic::catch_unwind(|| {
        let path_str = c_to_string_lossy(path);

        let mut repo = match Repository::open(&path_str) {
            Ok(r) => r,
            Err(e) => return make_err(&format!("git open failed: {}", e)),
        };

        match repo.stash_pop(0, None) {
            Ok(_) => make_ok_void(),
            Err(e) => make_err(&format!("git stash pop failed: {}", e)),
        }
    });
    result.unwrap_or_else(|_| make_panic_err())
}

// ============================================================================
// doo_git_has_remote — Check if any remote is configured
// ============================================================================

/// Returns 1 (true) if the repository has any remotes configured, 0 (false) otherwise.
/// Returns i32 for C ABI compatibility (matches Doo Bool → i32 at FFI boundary).
#[no_mangle]
pub extern "C" fn doo_git_has_remote(path: *const c_char) -> i32 {
    let result = panic::catch_unwind(|| {
        let path_str = c_to_string_lossy(path);

        let repo = match Repository::open(&path_str) {
            Ok(r) => r,
            Err(_) => return 0i32,
        };

        match repo.remotes() {
            Ok(remotes) => {
                if remotes.is_empty() {
                    0i32
                } else {
                    1i32
                }
            }
            Err(_) => 0i32,
        }
    });
    result.unwrap_or(0i32)
}

// ============================================================================
// doo_git_add_remote — Add or update a remote for a repository
// ============================================================================

/// Add a remote with the given name and URL to the repository.
/// If the remote already exists, update its URL instead.
/// Returns DooResult Ok(void) on success, Err on failure.
#[no_mangle]
pub extern "C" fn doo_git_add_remote(
    path: *const c_char,
    name: *const c_char,
    url: *const c_char,
) -> *mut DooResult {
    let result = panic::catch_unwind(|| {
        let path_str = c_to_string_lossy(path);
        let name_str = c_to_string_lossy(name);
        let url_str = c_to_string_lossy(url);

        let repo = match Repository::open(&path_str) {
            Ok(r) => r,
            Err(e) => return make_err(&format!("git open failed: {}", e)),
        };

        // Try to add; if remote exists, update URL instead
        let res = match repo.remote(&name_str, &url_str) {
            Ok(_remote) => make_ok_void(),
            Err(_) => {
                // Remote already exists — update its URL
                match repo.remote_set_url(&name_str, &url_str) {
                    Ok(_) => make_ok_void(),
                    Err(e) => make_err(&format!("git remote set-url failed: {}", e)),
                }
            }
        };
        res
    });
    result.unwrap_or_else(|_| make_panic_err())
}

// ============================================================================
// doo_git_head_short — Get the short HEAD commit hash
// ============================================================================

/// Return the first 7 characters of the HEAD commit hash.
#[no_mangle]
pub extern "C" fn doo_git_head_short(path: *const c_char) -> *mut DooResult {
    let result = panic::catch_unwind(|| {
        let path_str = c_to_string_lossy(path);

        let repo = match Repository::open(&path_str) {
            Ok(r) => r,
            Err(e) => return make_err(&format!("git open failed: {}", e)),
        };

        let head = match repo.head() {
            Ok(h) => h,
            Err(e) => return make_err(&format!("git head failed: {}", e)),
        };

        let result = match head.peel_to_commit() {
            Ok(commit) => {
                let full_hash = commit.id().to_string();
                let short = &full_hash[..std::cmp::min(7, full_hash.len())];
                make_ok_string(short)
            }
            Err(e) => make_err(&format!("git head commit failed: {}", e)),
        };
        result
    });
    result.unwrap_or_else(|_| make_panic_err())
}

// ============================================================================
// doo_git_current_branch — Get the current branch name
// ============================================================================

/// Return the name of the current branch (e.g. "main" or "master").
#[no_mangle]
pub extern "C" fn doo_git_current_branch(path: *const c_char) -> *mut DooResult {
    let result = panic::catch_unwind(|| {
        let path_str = c_to_string_lossy(path);

        let repo = match Repository::open(&path_str) {
            Ok(r) => r,
            Err(e) => return make_err(&format!("git open failed: {}", e)),
        };

        let head = match repo.head() {
            Ok(h) => h,
            Err(e) => return make_err(&format!("git head failed: {}", e)),
        };

        match head.shorthand() {
            Some(name) => make_ok_string(name),
            None => make_err("git: could not determine branch name"),
        }
    });
    result.unwrap_or_else(|_| make_panic_err())
}

// ============================================================================
// doo_git_commit_all_bg — Background commit (non-blocking)
// ============================================================================

/// Stage all changes + commit in a BACKGROUND THREAD.
/// Returns immediately with a content-based reference hash (7 chars).
/// The actual git add + commit runs asynchronously — caller doesn't wait.
///
/// This is the performance-critical path for DooCloud:
/// response returns instantly, git commit happens in background.
#[no_mangle]
pub extern "C" fn doo_git_commit_all_bg(
    path: *const c_char,
    message: *const c_char,
    author_name: *const c_char,
    author_email: *const c_char,
    push_remote: *const c_char,
    push_branch: *const c_char,
) -> *mut DooResult {
    let result = panic::catch_unwind(|| {
        let path_str = c_to_string_lossy(path);
        let msg_str = c_to_string_lossy(message);
        let name_str = c_to_string_lossy(author_name);
        let email_str = c_to_string_lossy(author_email);
        let remote_str = c_to_string_lossy(push_remote);
        let branch_str = c_to_string_lossy(push_branch);

        // Compute a quick reference hash for immediate return
        let mut hasher = DefaultHasher::new();
        msg_str.hash(&mut hasher);
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .hash(&mut hasher);
        let hash_val = hasher.finish();
        let ref_hash = format!("{:07x}", hash_val & 0x0FFFFFFF);

        // Spawn background thread for actual git operations (commit → push, in order)
        std::thread::spawn(move || {
            let op = || -> Result<(), git2::Error> {
                let repo = Repository::open(&path_str)?;

                // Stage all changes (git add -A)
                let mut index = repo.index()?;
                index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
                index.update_all(["*"].iter(), None)?;
                index.write()?;

                // Write tree
                let tree_oid = index.write_tree()?;
                let tree = repo.find_tree(tree_oid)?;

                // Signature
                let sig = Signature::now(&name_str, &email_str)?;

                // Parent commit (if any)
                let parent_commit = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
                let parents: Vec<&git2::Commit> = parent_commit.iter().collect();

                // Commit
                repo.commit(Some("HEAD"), &sig, &sig, &msg_str, &tree, &parents)?;

                // Push to remote (only if remote was provided)
                if !remote_str.is_empty() {
                    let target_branch = if branch_str.is_empty() {
                        "main".to_string()
                    } else {
                        branch_str.clone()
                    };
                    // Find remote by URL — first check if "origin" exists
                    if let Ok(mut remote) = repo.find_remote("origin") {
                        let refspec =
                            format!("refs/heads/{}:refs/heads/{}", target_branch, target_branch);
                        if let Err(e) = remote.push(&[&refspec], None) {
                            eprintln!("[doo_ffi_git] background push failed: {}", e);
                        }
                    }
                }

                Ok(())
            };

            if let Err(e) = op() {
                eprintln!("[doo_ffi_git] background commit failed: {}", e);
            }
        });

        // Return immediately with reference hash
        make_ok_string(&ref_hash)
    });
    result.unwrap_or_else(|_| make_panic_err())
}
