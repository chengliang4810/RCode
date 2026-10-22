import XCTest

/// Opt-in test against an authenticated direct engine and configured Native provider.
final class LiveStackTests: XCTestCase {
    func testSendRoundTripsThroughRealEngine() throws {
        let environment = ProcessInfo.processInfo.environment
        guard let host = environment["RCODE_LIVE_HOST"], !host.isEmpty,
              let apiKey = environment["RCODE_LIVE_API_KEY"], !apiKey.isEmpty else {
            throw XCTSkip("Set TEST_RUNNER_RCODE_LIVE_HOST and TEST_RUNNER_RCODE_LIVE_API_KEY for this online test")
        }
        let app = XCUIApplication()
        app.launchArguments = ["-signedout", "-connect", host, apiKey, "-harness", "native"]
        app.launch()

        let accessory = app.buttons["new-session"]
        XCTAssertTrue(accessory.waitForExistence(timeout: 15))
        accessory.tap()
        let input = app.textViews["composer-input"]
        XCTAssertTrue(input.waitForExistence(timeout: 5))
        input.typeText("hello from the phone")
        app.buttons["composer-send"].tap()

        // The host adopts the command and Native replies through the direct replica stream.
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
