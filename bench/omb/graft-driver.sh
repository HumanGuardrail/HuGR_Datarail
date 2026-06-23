#!/usr/bin/env bash
# Graft the datarail OMB driver into a cloned OpenMessaging Benchmark tree.
# Usage: graft-driver.sh <omb-src-dir> <driver-datarail-src-dir>
#
# Three edits, all idempotent:
#   1. copy the driver module into the tree
#   2. register it in the root pom's <modules>
#   3. declare it as a dependency of benchmark-framework — this is what bundles a driver JAR into the distro's
#      lib/ (package's assembly dependencySet pulls benchmark-framework's compile deps). Without it the class is
#      missing at runtime: ClassNotFoundException: DatarailBenchmarkDriver.
set -euo pipefail

OMB="$1"; DRIVER_SRC="$2"
cd "$OMB"

cp -r "$DRIVER_SRC" ./driver-datarail

# 2. root pom module (after driver-redis) — awk for portability (BSD/GNU)
if ! grep -q '<module>driver-datarail</module>' pom.xml; then
  awk '{ print }
       /<module>driver-redis<\/module>/ { print "        <module>driver-datarail</module>" }' \
    pom.xml > pom.xml.new
  mv pom.xml.new pom.xml
fi

# 3. benchmark-framework dependency (insert a block right after the driver-redis <dependency>…</dependency>)
if ! grep -q '<artifactId>driver-datarail</artifactId>' benchmark-framework/pom.xml; then
  awk '
    { print }
    /<artifactId>driver-redis<\/artifactId>/ { inredis = 1 }
    inredis && /<\/dependency>/ {
      print "        <dependency>"
      print "            <groupId>${project.groupId}</groupId>"
      print "            <artifactId>driver-datarail</artifactId>"
      print "            <version>${project.version}</version>"
      print "        </dependency>"
      inredis = 0
    }
  ' benchmark-framework/pom.xml > benchmark-framework/pom.xml.new
  mv benchmark-framework/pom.xml.new benchmark-framework/pom.xml
fi

echo "graft OK:"
echo "  module registered : $(grep -c '<module>driver-datarail</module>' pom.xml)"
echo "  framework dep      : $(grep -c '<artifactId>driver-datarail</artifactId>' benchmark-framework/pom.xml)"
