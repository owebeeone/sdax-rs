"""Read crates.io state; a network error is never evidence of absence."""

import hashlib
import json
import tomllib
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

from package_archives import read_archive
from release_selection import ReleaseError


def request(url, *, absent_ok=False):
    try:
        with urlopen(Request(url, headers={"User-Agent": "sdax-release/1.0"}), timeout=30) as response:
            return response.read()
    except HTTPError as error:
        error.close()
        if error.code == 404 and absent_ok:
            return None
        raise ReleaseError(f"registry HTTP {error.code}: {url}") from error
    except (URLError, OSError) as error:
        raise ReleaseError(f"registry request failed: {url}: {error}") from error


class Registry:
    def __init__(self, expected_archives):
        self.expected = expected_archives

    def existing(self, crate, selected):
        """Return a verified archive checksum, or None for a confirmed 404.

        Cargo's VCS metadata is self-reported. Also compare the complete packaged
        source against a freshly generated archive of the selected checkout.
        The generated lockfile may differ after registry indexing; its adapter
        core version is checked separately.
        """
        base = f"https://crates.io/api/v1/crates/{crate}/{selected.version}"
        response = request(base, absent_ok=True)
        if response is None:
            return None
        try:
            meta = json.loads(response)["version"]
            if (meta["crate"], meta["num"], meta["yanked"]) != (crate, selected.version, False):
                raise ReleaseError(f"unexpected or yanked registry version for {crate}")
            data = request(base + "/download")
            checksum = hashlib.sha256(data).hexdigest()
            if checksum != meta["checksum"]:
                raise ReleaseError(f"registry archive checksum mismatch for {crate}")
            files = read_archive(data, crate, selected.version)
            provenance = json.loads(files[".cargo_vcs_info.json"])
            if (provenance["git"]["sha1"] != selected.sha
                    or provenance["git"].get("dirty", False) is not False
                    or provenance["path_in_vcs"] != f"crates/{crate}"):
                raise ReleaseError(f"registry archive provenance mismatch for {crate}")
            expected = read_archive(self.expected[crate], crate, selected.version)
            generated = {"Cargo.lock", ".cargo_vcs_info.json"}
            sources = lambda archive: {name: data for name, data in archive.items() if name not in generated}
            if sources(files) != sources(expected):
                raise ReleaseError(f"registry archive source differs from selected checkout for {crate}")
            if crate == "sdax-tokio":
                lock = tomllib.loads(files["Cargo.lock"].decode())
                core_versions = [pkg["version"] for pkg in lock["package"] if pkg["name"] == "sdax"]
                if core_versions != [selected.version]:
                    raise ReleaseError("existing adapter does not resolve the selected core version")
            return checksum
        except (KeyError, ValueError, TypeError) as error:
            raise ReleaseError(f"invalid registry evidence for {crate}: {error}") from error

    def core_indexed(self, version, checksum):
        data = request("https://index.crates.io/sd/ax/sdax", absent_ok=True)
        if data is None:
            return False
        try:
            matches = [row for line in data.splitlines()
                       if (row := json.loads(line))["vers"] == version]
            if not matches:
                return False
            if (len(matches) != 1 or matches[0]["cksum"] != checksum
                    or matches[0]["yanked"] is not False):
                raise ReleaseError("core registry index differs from verified release archive")
            return True
        except (KeyError, ValueError, TypeError) as error:
            raise ReleaseError(f"invalid core registry index: {error}") from error
