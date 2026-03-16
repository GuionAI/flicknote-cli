package main

import (
	"context"
	"fmt"
	"strings"

	"dagger/flicknote-cli/internal/dagger"
)

type FlicknoteCli struct{}

// Build produces a minimal container image with the static flicknote binary.
// Uses rust:bookworm (glibc) to cross-compile to musl target — bindgen
// needs dlopen for libclang which doesn't work on Alpine/musl.
// Syncs the builder stage to surface compile errors early (Dagger uses lazy eval).
func (m *FlicknoteCli) Build(ctx context.Context, source *dagger.Directory) (*dagger.Container, error) {
	// Use glibc-based image for building — bindgen needs dlopen for libclang,
	// which doesn't work on musl (Alpine). Cross-compile to musl target instead.
	builder, err := dag.Container().
		From("rust:bookworm").
		WithExec([]string{"apt-get", "update"}).
		WithExec([]string{"apt-get", "install", "-y", "--no-install-recommends", "musl-tools", "libclang-dev"}).
		WithDirectory("/app", source).
		WithWorkdir("/app").
		WithExec([]string{
			"cargo", "build", "--release",
			"-p", "flicknote-cli",
			"-p", "flicktask-cli",
			"--no-default-features",
			"--target", "x86_64-unknown-linux-musl",
		}).
		Sync(ctx)
	if err != nil {
		return nil, fmt.Errorf("build flicknote-cli: %w", err)
	}

	// Minimal image — binaries only, used as a copy source by other builds
	return dag.Container().
		From("alpine:3.23").
		WithFile(
			"/usr/local/bin/flicknote",
			builder.File("/app/target/x86_64-unknown-linux-musl/release/flicknote"),
		).
		WithFile(
			"/usr/local/bin/flicktask",
			builder.File("/app/target/x86_64-unknown-linux-musl/release/flicktask"),
		).
		WithExec([]string{"chmod", "+x", "/usr/local/bin/flicknote", "/usr/local/bin/flicktask"}), nil
}

// Publish builds and pushes the image to the registry with the given tags.
func (m *FlicknoteCli) Publish(
	ctx context.Context,
	source *dagger.Directory,
	registry string,
	image string,
	tags string,
) (string, error) {
	if registry == "" {
		return "", fmt.Errorf("registry must not be empty")
	}
	if image == "" {
		return "", fmt.Errorf("image must not be empty")
	}

	tagList := strings.Split(tags, ",")
	var validTags []string
	for _, t := range tagList {
		if t = strings.TrimSpace(t); t != "" {
			validTags = append(validTags, t)
		}
	}
	if len(validTags) == 0 {
		return "", fmt.Errorf("no valid tags provided")
	}

	container, err := m.Build(ctx, source)
	if err != nil {
		return "", err
	}

	var published []string
	for _, tag := range validTags {
		ref := fmt.Sprintf("%s/%s:%s", registry, image, tag)
		pushedRef, err := container.Publish(ctx, ref)
		if err != nil {
			return "", fmt.Errorf("published %v, failed at %s: %w", published, ref, err)
		}
		published = append(published, pushedRef)
	}

	return published[len(published)-1], nil
}
