#!/bin/sh
set -e
(which git  && which mdbook && which rsync ) > /dev/null || (echo "Cannot find 'git', 'rsync', or 'mdbook' in PATH" && exit 5)

workdir=$(mktemp -d)
sourcetree="$workdir"/source
mdbook_root="$sourcetree/docs/.mdbook"
# TODO: Have a separate `outtree` and `gh-pages`
# to refresh the world each time
outtree="$workdir"/gh-pages

git worktree add "$sourcetree" --detach --quiet
git worktree add "$outtree" -f gh-pages --quiet
# git -C "$outtree" rm -r "$outtree/"

cd "$sourcetree" || exit 5
printf "# \`jj\` documentation\nPick a version of \`jj\` to view the documentation:\n\n" > "$outtree/index.md"
for tag in mdbook $(git tag | grep -E 'v[0-9]+\.[0-9]+(\.[0-9]+)?(-mdbook)?' | sort -r); do  # REPLACE mdbook WITH main
  git switch --detach "$tag" --quiet

  # Lie. 
  case "$tag" in
     mdbook|main)  # DELETE mdbook after merging
       tag=main
       friendly_tag='prerelease (main branch)';;
     v0.8.0-mdbook) # Actual v0.8.0 doesn't have SUMMARY.md
       tag=v0.8.0
       friendly_tag="v0.8.0 stable";;
     *)
       friendly_tag="$tag stable";;
  esac

  if [ -r "$mdbook_root/book.toml" ]; then
    cd "$mdbook_root" || exit 5
    sed -i "s/^title\s*=.*/title =\"jj $friendly_tag docs\"/" book.toml
    mdbook build
    rsync -rI --delete "$sourcetree/rendered-docs/" "$outtree/$tag/"
    echo "- [$friendly_tag]($tag/index.html)" >> "$outtree/index.md"
    git restore :/ --quiet
  fi
done
cd "$outtree" || exit 5
ls
git add .
git commit  # TODO: some message author

