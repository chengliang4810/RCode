import UIKit
import XCTest
@testable import RCode

final class QuestionPanelTests: XCTestCase {
    @MainActor
    private func control<T: UIView>(_ type: T.Type, id: String, in view: UIView) -> T? {
        if view.accessibilityIdentifier == id, let match = view as? T { return match }
        return view.subviews.lazy.compactMap { self.control(type, id: id, in: $0) }.first
    }

    @MainActor
    func testDuplicateLabelsKeepOptionIdsSeparateFromFreeText() throws {
        let panel = QuestionPanel()
        var submitted: [UserInputAnswer] = []
        var submissions = 0
        panel.onSubmit = { submitted = $0; submissions += 1 }
        panel.configure([SessionChrome.Question(
            id: "choice", header: "", text: "Choose",
            options: [QuestionOption(id: "first", label: "Same"), QuestionOption(id: "second", label: "Same")],
            multiSelect: true
        )])
        let second = try XCTUnwrap(control(UIButton.self, id: "question-option-1", in: panel))
        second.sendActions(for: .touchUpInside)
        let other = try XCTUnwrap(control(UITextField.self, id: "question-other", in: panel))
        other.text = "Extra detail"
        other.sendActions(for: .editingChanged)
        let submit = try XCTUnwrap(control(UIButton.self, id: "question-next", in: panel))
        submit.sendActions(for: .touchUpInside)
        XCTAssertEqual(submitted, [UserInputAnswer(questionId: "choice", selectedOptionIds: ["second"], text: "Extra detail")])
        submit.sendActions(for: .touchUpInside)
        XCTAssertEqual(submissions, 1)
    }

    @MainActor
    func testMultilinePrefillRemainsFreeText() throws {
        let panel = QuestionPanel()
        var submitted: [UserInputAnswer] = []
        panel.onSubmit = { submitted = $0 }
        panel.configure([SessionChrome.Question(
            id: "text", header: "", text: "Details", options: [], multiSelect: false,
            prefill: "First line\nSecond line", multiline: true
        )])
        let submit = try XCTUnwrap(control(UIButton.self, id: "question-next", in: panel))
        submit.sendActions(for: .touchUpInside)
        XCTAssertEqual(submitted, [UserInputAnswer(questionId: "text", selectedOptionIds: [], text: "First line\nSecond line")])
    }
}
