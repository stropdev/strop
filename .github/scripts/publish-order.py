#!/usr/bin/env python3
"""Read cargo metadata from stdin; emit publishable workspace crates in dependency order."""
import json
import sys


def publication_order(metadata):
    members = set(metadata["workspace_members"])
    packages = {package["name"]: package for package in metadata["packages"]
                if package["id"] in members}
    graph = {}
    for name, package in packages.items():
        if package.get("publish") == []:
            continue
        dependencies = {
            dependency["name"] for dependency in package["dependencies"]
            if dependency["kind"] != "dev" and dependency.get("path") is not None
            and dependency["name"] in packages
        }
        private = [dependency for dependency in dependencies
                   if packages[dependency].get("publish") == []]
        if private:
            raise ValueError(f"{name} depends on unpublished workspace crates: {sorted(private)}")
        graph[name] = dependencies
    ordered = []
    published = set()
    while graph:
        ready = sorted(name for name, dependencies in graph.items() if dependencies <= published)
        if not ready:
            raise ValueError(f"workspace dependency cycle: {sorted(graph)}")
        for name in ready:
            ordered.append(name)
            published.add(name)
            del graph[name]
    return ordered


if __name__ == "__main__":
    try:
        order = publication_order(json.load(sys.stdin))
    except (ValueError, KeyError, TypeError) as error:
        sys.exit(f"invalid publication graph: {error}")
    print("\n".join(order))
