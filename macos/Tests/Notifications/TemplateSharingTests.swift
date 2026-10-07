import XCTest
@testable import Lorca

final class TemplateSharingTests: XCTestCase {
    func testPreviewContainsEverySelectedResourceAndReviewWarning() {
        let preview = TemplatePreview(json: ["digest": "reviewed", "template": [
            "profile": ["name": "Reviewer", "description": "Read carefully", "symbol_name": "sparkles", "accent": "blue"],
            "skills": [["name": "review", "description": "Review code", "instructions": "Check changes", "examples": "Read diff", "references": [["path": "references/checklist.md", "text": "Check errors"]], "scripts": [["path": "scripts/check.sh", "text": "echo check"]]]],
            "memories": ["Use short replies"],
            "routines": [["name": "Morning", "schedule": "every 2h", "prompt": "Read inbox", "check": "return true;", "timezone": "America/New_York", "missed_run_policy": "skip"]],
            "requirements": [["service_id": "google-drive"]]
        ], "warnings": [["path": "memories.0", "message": "May be personal"]]])
        for content in ["Reviewer", "Read carefully", "Check changes", "Read diff", "references/checklist.md", "Check errors", "scripts/check.sh", "echo check", "Use short replies", "Morning", "return true;", "America/New_York", "skip", "google-drive", "May be personal"] {
            XCTAssertTrue(preview.text.contains(content), content)
        }
        XCTAssertEqual(preview.digest, "reviewed")
    }

    func testRecipientServicesAndAccountIDsRemainExact() {
        let id = "google-drive-0123456789abcdef0123456789abcdef"
        let preview = TemplatePreview(json: ["can_import": false, "issues": ["Select your connection"], "digest": "v1", "requirements": [["service_id": "google-drive", "candidates": [["id": id, "name": "Drive · Personal", "state": "ready", "detail": "Ready"]]]]])
        XCTAssertFalse(preview.canImport)
        XCTAssertEqual(preview.requirements[0].serviceID, "google-drive")
        XCTAssertEqual(preview.requirements[0].candidates[0].id, id)
        XCTAssertEqual(preview.issues, ["Select your connection"])
    }

    func testContentsOfferOnlyExplicitSelectionIDs() {
        let contents = TemplateContents(json: ["profile": ["name": "Source"], "skills": [["id": "playbook-source", "content": ["name": "review"]]], "memories": [["id": "memory-hash", "content": "Selected fact"]], "routines": [["id": "routine-source", "content": ["name": "Morning"]]], "requirements": [["service_id": "gmail"]], "notes": ["Capability missing"]])
        XCTAssertEqual(contents.skills.map(\.id), ["playbook-source"])
        XCTAssertEqual(contents.memories.map(\.title), ["Selected fact"])
        XCTAssertEqual(contents.requirements.map(\.id), ["gmail"])
        XCTAssertEqual(contents.notes, ["Capability missing"])
    }
}
