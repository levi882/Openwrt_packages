import hashlib
import importlib.util
import io
import json
import tempfile
import unittest
import urllib.error
from pathlib import Path
from unittest.mock import patch


def load_script(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


UPDATER = load_script("update-release-apks")
DOWNLOADER = load_script("download-release-apks")
BROWSER_URL = "https://github.com/owner/repo/releases/download/v1/example.apk"
API_URL = "https://api.github.com/repos/owner/repo/releases/assets/123"


def http_error(code):
    return urllib.error.HTTPError(BROWSER_URL, code, "test error", {}, io.BytesIO())


class UpdaterRequestTests(unittest.TestCase):
    def test_retries_gateway_error_then_returns_payload(self):
        with patch.object(UPDATER.urllib.request, "urlopen", side_effect=[http_error(504), io.BytesIO(b"payload")]) as open_url, \
             patch.object(UPDATER.time, "sleep") as sleep:
            self.assertEqual(UPDATER.github_request(BROWSER_URL), b"payload")
        self.assertEqual(open_url.call_count, 2)
        sleep.assert_called_once_with(1)

    def test_does_not_retry_invalid_credentials(self):
        with patch.object(UPDATER.urllib.request, "urlopen", side_effect=http_error(401)) as open_url, \
             patch.object(UPDATER.time, "sleep") as sleep:
            with self.assertRaises(urllib.error.HTTPError):
                UPDATER.github_request(API_URL)
        self.assertEqual(open_url.call_count, 1)
        sleep.assert_not_called()

    def test_retry_count_is_bounded(self):
        with patch.object(UPDATER.urllib.request, "urlopen", side_effect=[http_error(503) for _ in range(3)]) as open_url, \
             patch.object(UPDATER.time, "sleep"):
            with self.assertRaises(urllib.error.HTTPError):
                UPDATER.github_request(BROWSER_URL)
        self.assertEqual(open_url.call_count, 3)

    def test_asset_api_fallback_requests_binary(self):
        asset = {"name": "example.apk", "browser_download_url": BROWSER_URL, "url": API_URL}
        with patch.object(UPDATER, "github_request", side_effect=[http_error(504), b"apk"]) as request:
            self.assertEqual(UPDATER.download_asset(asset), b"apk")
        self.assertEqual(request.call_args_list[1].args, (API_URL,))
        self.assertEqual(request.call_args_list[1].kwargs, {"accept": "application/octet-stream"})


class ManifestDownloadTests(unittest.TestCase):
    def exercise_fallback(self, expected_sha):
        metadata = json.dumps({"assets": [{"name": "example.apk", "url": API_URL}]}).encode()
        with tempfile.TemporaryDirectory() as temp:
            target = Path(temp) / "example.apk"
            with patch.object(DOWNLOADER.urllib.request, "urlopen", side_effect=[http_error(504), io.BytesIO(metadata), io.BytesIO(b"apk")]) as open_url, \
                 patch.object(DOWNLOADER.time, "sleep"):
                DOWNLOADER.download(BROWSER_URL, target, expected_sha)
                self.assertEqual(target.read_bytes(), b"apk")
            self.assertEqual(open_url.call_args_list[2].args[0].full_url, API_URL)
            self.assertEqual(open_url.call_args_list[2].args[0].get_header("Accept"), "application/octet-stream")

    def test_api_fallback_download_passes_hash_check(self):
        self.exercise_fallback(hashlib.sha256(b"apk").hexdigest())

    def test_api_fallback_keeps_hash_mismatch_fatal(self):
        with self.assertRaisesRegex(SystemExit, "SHA256 mismatch"):
            self.exercise_fallback("0" * 64)

    def test_fallback_only_uses_github_release_urls(self):
        with patch.object(DOWNLOADER.urllib.request, "urlopen") as open_url:
            self.assertIsNone(DOWNLOADER.github_asset_api_url("https://example.org/example.apk"))
        open_url.assert_not_called()


if __name__ == "__main__":
    unittest.main()
