#!/bin/bash

set -eu -o pipefail

changelog=$(cat CHANGELOG.md)

regex='## \[([0-9]+\.[0-9]+\.[0-9]+)\] - ([0-9]{4}-[0-9]{2}-[0-9]{2})

((.|
)*)'

if [[ ! $changelog =~ $regex ]]; then
    echo "Could not find version and date line in change log!"
    exit 1
fi

version="${BASH_REMATCH[1]}"
date="${BASH_REMATCH[2]}"
notes_raw="${BASH_REMATCH[3]}"

# Extract notes until the next version heading or end.
notes="$(echo "$notes_raw" | sed -n -e '/^## \[/q;p')"

if [[ "$date" !=  $(date +"%Y-%m-%d") ]]; then
    echo "$date is not today!"
    exit 1
fi

tag="v$version"

if [ -n "$(git status --porcelain)" ]; then
    echo ". is not clean." >&2
    exit 1
fi

package_version="$(node -p "require('./package.json').version")"
if [[ "$package_version" != "$version" ]]; then
    npm version "$version" --no-git-tag-version
fi

cargo_version="$(perl -ne 'if (/^version = "([^"]+)"/) { print $1; exit }' Cargo.toml)"
if [[ "$cargo_version" != "$version" ]]; then
    perl -pi -e "s/(?<=^version = \").+?(?=\")/$version/gsm" Cargo.toml
    perl -pi -e "s/(?<=maxmind\.nativeVersion\(\), ').+?(?=')/$version/gsm" test/basic.test.js
fi

echo $"Test results:"
npm ci
cargo fmt -- --check
cargo check
cargo clippy --all-targets -- -D warnings
npm run build
npm test
npm run typecheck
npm run test:pack
npm pack --dry-run

echo $'\nDiff:'
git diff

echo $'\nRelease notes:'
echo "$notes"

read -e -p "Commit changes and push to origin? " should_push

if [ "$should_push" != "y" ]; then
    echo "Aborting"
    exit 1
fi

if [ -n "$(git status --porcelain)" ]; then
    git commit -m "Update for $tag" package.json package-lock.json Cargo.toml Cargo.lock test/basic.test.js
fi

git push

gh release create --target "$(git branch --show-current)" -t "$version" -n "$notes" "$tag"

git push --tags
