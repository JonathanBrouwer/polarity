#!/usr/bin/env bash
test -e xfunc_artifact.zip && rm xfunc_artifact.zip

DIR=$(mktemp -d)
git clone --depth 1 $(git remote get-url origin) $DIR/

rm $DIR/scripts/package_artifact.sh
rm $DIR/scripts/oopsla_snippets.sh

# only keep whitelisted files in oopsla_examples/, then delete whitelist
for file in oopsla_examples/*.xfn; do
    grep -q $(basename $file) oopsla_examples/whitelist.txt || rm $DIR/$file
done
rm $DIR/oopsla_examples/whitelist.txt

# remove potential build artifacts
rm -r $DIR/target/

rm -rf $DIR/.git/
rm -rf $DIR/.gitignore
rm -rf $DIR/.github/
rm -rf $DIR/.cargo/

pushd $DIR
zip -r xfunc_artifact.zip .
popd
mv $DIR/xfunc_artifact.zip .
rm -rf $DIR
