package main

import (
	"context"
	"fmt"
	"strings"

	"dagger/flicknote-cli/internal/dagger"
)

type FlicknoteCli struct{}

// Build produces a minimal container image with the static flicknote binary.
// Uses rust:alpine (musl libc) — produces a fully static binary.
// Syncs the builder stage to surface compile errors early (Dagger uses lazy eval).
func (m *FlicknoteCli) Build(ctx context.Context, source *dagger.Directory) (*dagger.Container, error) {
	builder, err := dag.Container().
		From("rust:alpine").
		WithExec([]string{"apk", "add", "--no-cache", "musl-dev", "build-base"}).
		WithExec([]string{"rustup", "target", "add", "x86_64-unknown-linux-musl"}).
		WithDirectory("/app", source).
		WithWorkdir("/app").
		WithExec([]string{
			"cargo", "build", "--release",
			"-p", "flicknote-cli",
			"--target", "x86_64-unknown-linux-musl",
		}).
		Sync(ctx)
	if err != nil {
		return nil, fmt.Errorf("build flicknote-cli: %w", err)
	}

	// Minimal image — binary only, used as a copy source by other builds
	return dag.Container().
		From("alpine:3.21").
		WithFile(
			"/usr/local/bin/flicknote",
			builder.File("/app/target/x86_64-unknown-linux-musl/release/flicknote"),
		).
		WithExec([]string{"chmod", "+x", "/usr/local/bin/flicknote"}), nil
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
