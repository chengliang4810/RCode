import XCTest

/// Opt-in test against a supplied registry/session host and a configured Native
/// provider. See README.md. No hosted backend is bundled in this repository.
final class LiveStackTests: XCTestCase {
    func testSendRoundTripsThroughRealEngine() throws {
        let environment = ProcessInfo.processInfo.environment
        guard let host = environment["RCODE_LIVE_HOST"], !host.isEmpty,
              let token = environment["RCODE_LIVE_PAIR_TOKEN"], !token.isEmpty else {
            throw XCTSkip("Set TEST_RUNNER_RCODE_LIVE_HOST and TEST_RUNNER_RCODE_LIVE_PAIR_TOKEN for this online test")
        }
        let identity = token.split(separator: "@", maxSplits: 1, omittingEmptySubsequences: false)
        guard identity.count == 2, identity.allSatisfy({ !$0.isEmpty }) else {
            XCTFail("Pairing token must contain a user and organization")
            return
        }
        let app = XCUIApplication()
        app.launchArguments = ["-signedout", "-pair", host, String(identity[0]), String(identity[1]), "-harness", "native"]
        app.launch()

        let accessory = app.buttons["new-session"]
        XCTAssertTrue(accessory.waitForExistence(timeout: 15))
        accessory.tap()
        let input = app.textViews["composer-input"]
        XCTAssertTrue(input.waitForExistence(timeout: 5))
        input.typeText("hello from the phone")
        app.buttons["composer-send"].tap()

        // The host adopts the command and Native replies through the session room.
        let echo = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'You: hello from the phone'")).firstMatch
        XCTAssertTrue(echo.waitForExistence(timeout: 10))
        let reply = app.staticTexts.matching(identifier: "row-markdown").firstMatch
        XCTAssertTrue(reply.waitForExistence(timeout: 60), "Native provider reply streamed back")
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = "live-roundtrip"
        shot.lifetime = .keepAlways
        add(shot)
        XCTAssertTrue(app.buttons["Send message"].waitForExistence(timeout: 60), "turn settles")
    }
}
