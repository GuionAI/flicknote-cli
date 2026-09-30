package main

import (
	"context"
	"fmt"

	"dagger/flicknote-cli-check/internal/dagger"
)

const pgImage = "ghcr.io/guionai/cloudnative-supabase-postgres-pgroonga@sha256:d555bf68fad60626664e22cf90390fc8936b42092d504f7f02a592ec70b92626"

type FlicknoteCliCheck struct{}

// Check Rust and real MCP behavior in a disposable migration-backed PG container.
func (m *FlicknoteCliCheck) Check(ctx context.Context,
	// CLI checkout; omit credentials and local build/state directories.
	// +ignore=[".git", ".scratch", "target", "**/.env"]
	source *dagger.Directory,
	// Optional read-only fb checkout for local reproduction.
	// +ignore=["**/node_modules", ".scratch", "**/.env", "**/target", ".turbo"]
	// +optional
	fbSource *dagger.Directory,
	// Exact fb revision to replay; empty resolves main once.
	// +optional
	fbRevision string,
	// Read-only private fb access, if the runner requires it.
	// +optional
	fbToken *dagger.Secret,
) (string, error) {
	if fbSource == nil {
		repo := dag.Git("http://forgejo.devops.svc.cluster.local:3000/GuionAI/flick-backend.git", dagger.GitOpts{HTTPAuthUsername: "x-access-token", HTTPAuthToken: fbToken})
		ref := repo.Branch("main")
		if fbRevision != "" {
			ref = repo.Commit(fbRevision)
		}
		commit, err := ref.Commit(ctx)
		if err != nil {
			return "", fmt.Errorf("resolve fb source (runner needs read access): %w", err)
		}
		fmt.Printf("FB_COMMIT=%s PG_IMAGE=%s\n", commit, pgImage)
		fbSource = repo.Commit(commit).Tree()
		fbRevision = commit
	}
	rust := dag.Container().From("docker.io/library/rust:1.96.1-bookworm").
		WithExec([]string{"rustup", "component", "add", "rustfmt", "clippy"})
	dbmate := dag.Container().From("ghcr.io/amacneil/dbmate:2.33.0")
	check := dag.Container().From(pgImage).WithUser("root").
		WithExec([]string{"apt-get", "update"}).
		WithExec([]string{"apt-get", "install", "-y", "--no-install-recommends", "build-essential", "cmake", "pkg-config", "clang", "libclang-dev", "git", "ca-certificates"}).
		WithDirectory("/usr/local/rustup", rust.Directory("/usr/local/rustup")).
		WithDirectory("/usr/local/cargo", rust.Directory("/usr/local/cargo")).
		WithFile("/usr/local/bin/dbmate", dbmate.File("/usr/local/bin/dbmate")).
		WithUser("postgres").
		WithEnvVariable("PATH", "/usr/local/cargo/bin:/usr/lib/postgresql/18/bin:/usr/local/bin:/usr/bin:/bin").
		WithEnvVariable("RUSTUP_HOME", "/usr/local/rustup").
		WithEnvVariable("RUSTUP_TOOLCHAIN", "1.96.1").
		WithEnvVariable("CARGO_HOME", "/tmp/cargo").
		WithMountedCache("/tmp/cargo/registry", dag.CacheVolume("fn-cli-registry-rust196"), dagger.ContainerWithMountedCacheOpts{Owner: "postgres", Sharing: dagger.CacheSharingModeLocked}).
		WithMountedCache("/tmp/cargo/git", dag.CacheVolume("fn-cli-git-rust196"), dagger.ContainerWithMountedCacheOpts{Owner: "postgres", Sharing: dagger.CacheSharingModeLocked}).
		WithMountedCache("/src/target", dag.CacheVolume("fn-cli-target-rust196-pg18"), dagger.ContainerWithMountedCacheOpts{Owner: "postgres", Sharing: dagger.CacheSharingModeLocked}).
		WithDirectory("/src", source, dagger.ContainerWithDirectoryOpts{Owner: "postgres"}).
		WithDirectory("/fb", fbSource, dagger.ContainerWithDirectoryOpts{Owner: "postgres"}).
		WithWorkdir("/src").
		WithEnvVariable("FLICKNOTE_TEST_FB_CHECKOUT", "/fb").
		WithEnvVariable("FLICKNOTE_TEST_PG_IN_CONTAINER", "1").
		WithEnvVariable("FLICKNOTE_TEST_PG_IMAGE", pgImage)
	if fbRevision != "" {
		check = check.WithEnvVariable("FLICKNOTE_TEST_FB_REVISION", fbRevision)
	}
	return check.WithExec([]string{"cargo", "fmt", "--all", "--check"}).
		WithExec([]string{"bash", "scripts/test-release.sh"}).
		WithExec([]string{"cargo", "test", "--workspace", "--all-features", "--locked"}).
		WithExec([]string{"cargo", "clippy", "--workspace", "--all-targets", "--all-features", "--locked", "--", "-D", "warnings"}).
		WithExec([]string{"bash", "scripts/test-private-pg.sh"}).Stdout(ctx)
}
